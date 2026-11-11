use std::net::SocketAddr;
use std::time::Duration;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use paracord_shared::{
    AuthRequest, AuthResponse, ServerEvent, UpdateMe, User, check_password, clean_display_name,
    normalize_username,
};

use super::communities::create_community;
use crate::access::{community_member_ids, home_community, related_user_ids};
use crate::auth::{
    CurrentUser, USER_COLUMNS, UserRow, clear_cookie, create_session, dummy_hash, hash_password,
    session_cookie, session_token, to_user, verify_password,
};
use crate::error::{ApiError, ApiResult, is_duplicate};
use crate::state::AppState;
use crate::util::{client_ip, is_https, new_id, now_ms, random_hue, sha256_hex};

fn limit_auth(state: &AppState, headers: &HeaderMap, peer: &SocketAddr) -> ApiResult<()> {
    let ip = client_ip(&state.cfg, headers, peer);
    if !state.allow(&format!("auth:{ip}"), 10, Duration::from_secs(5 * 60)) {
        return Err(ApiError::TooManyRequests);
    }
    Ok(())
}

pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<AuthRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, &headers, &peer)?;
    let username = normalize_username(&body.username).map_err(ApiError::bad)?;
    check_password(&body.password).map_err(ApiError::bad)?;
    let display_name = match body.display_name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => clean_display_name(name).map_err(ApiError::bad)?,
        _ => username.clone(),
    };
    let taken = || ApiError::Conflict("That username is taken.".into());

    let mut c = state.conn().await?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
        .bind(&username)
        .fetch_optional(&mut *c)
        .await?;
    if exists.is_some() {
        return Err(taken());
    }
    drop(c);

    let password = body.password;
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(ApiError::internal)??;

    let user = User {
        id: new_id(),
        username,
        display_name,
        avatar_hue: random_hue(),
    };
    let now = now_ms();
    let mut tx = state.pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, avatar_hue, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.username)
    .bind(&user.display_name)
    .bind(&hash)
    .bind(user.avatar_hue)
    .bind(now)
    .execute(&mut *tx)
    .await;
    match inserted {
        Err(error) if is_duplicate(&error) => return Err(taken()),
        other => other?,
    };
    // Everyone joins the first community. The very first account creates it.
    let home = match home_community(&mut tx).await? {
        Some(id) => {
            sqlx::query("INSERT IGNORE INTO members (community_id, user_id, joined_at) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(&user.id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            id
        }
        None => create_community(&mut tx, &user.id, "Paracord", &["general", "introductions"]).await?,
    };
    let token = create_session(&mut tx, &user.id).await?;
    tx.commit().await?;

    let mut c = state.conn().await?;
    let members = community_member_ids(&mut c, &home).await?;
    state.hub.send(&members, &ServerEvent::MembersChanged { community_id: home });

    let cookie = session_cookie(&token, is_https(&state.cfg, &headers, &peer));
    Ok((StatusCode::CREATED, [(SET_COOKIE, cookie)], Json(AuthResponse { user })).into_response())
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<AuthRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, &headers, &peer)?;
    let username = normalize_username(&body.username).unwrap_or_default();

    let found: Option<(String, String, String, i64, String)> = sqlx::query_as(sql!(
        "SELECT {USER_COLUMNS}, u.password_hash FROM users u WHERE u.username = ?"
    ))
    .bind(&username)
    .fetch_optional(&state.pool)
    .await?;

    let password = body.password;
    let stored = found
        .as_ref()
        .map(|row| row.4.clone())
        .unwrap_or_else(|| dummy_hash().to_string());
    let ok = tokio::task::spawn_blocking(move || verify_password(&password, &stored))
        .await
        .map_err(ApiError::internal)?;
    let user = match found {
        Some((id, username, display_name, hue, _)) if ok => to_user((id, username, display_name, hue) as UserRow),
        _ => return Err(ApiError::bad("That username and password don't match.")),
    };

    let mut c = state.conn().await?;
    let token = create_session(&mut c, &user.id).await?;
    let cookie = session_cookie(&token, is_https(&state.cfg, &headers, &peer));
    Ok(([(SET_COOKIE, cookie)], Json(AuthResponse { user })).into_response())
}

pub async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if let Some(token) = session_token(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(sha256_hex(&token))
            .execute(&state.pool)
            .await?;
    }
    let cookie = clear_cookie(is_https(&state.cfg, &headers, &peer));
    Ok(([(SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response())
}

pub async fn me(CurrentUser(user): CurrentUser) -> Json<User> {
    Json(user)
}

pub async fn update_me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<UpdateMe>,
) -> ApiResult<Json<User>> {
    let display_name = clean_display_name(&body.display_name).map_err(ApiError::bad)?;
    let updated = User { display_name, ..user };
    let mut c = state.conn().await?;
    sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
        .bind(&updated.display_name)
        .bind(&updated.id)
        .execute(&mut *c)
        .await?;
    let audience = related_user_ids(&mut c, &updated.id).await?;
    state.hub.send(&audience, &ServerEvent::UserUpdated { user: updated.clone() });
    Ok(Json(updated))
}
