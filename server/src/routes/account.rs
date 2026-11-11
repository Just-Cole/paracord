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
use rusqlite::OptionalExtension;

use super::communities::create_community;
use crate::access::{community_member_ids, related_user_ids};
use crate::auth::{
    CurrentUser, clear_cookie, create_session, dummy_hash, hash_password, session_cookie,
    session_token, verify_password,
};
use crate::error::{ApiError, ApiResult};
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
    let password = body.password;
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(ApiError::internal)??;

    let (user, token, home) = state
        .db(move |c| {
            let tx = c.transaction()?;
            let taken = tx
                .query_row("SELECT 1 FROM users WHERE username = ?1", [&username], |_| Ok(()))
                .optional()?
                .is_some();
            if taken {
                return Err(ApiError::Conflict("That username is taken.".into()));
            }
            let user = User {
                id: new_id(),
                username,
                display_name,
                avatar_hue: random_hue(),
            };
            let now = now_ms();
            tx.execute(
                "INSERT INTO users (id, username, display_name, password_hash, avatar_hue, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (&user.id, &user.username, &user.display_name, &hash, user.avatar_hue, now),
            )?;
            // Everyone joins the first community. The very first account creates it.
            let home: Option<String> = tx
                .query_row(
                    "SELECT id FROM communities ORDER BY created_at, id LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let home = match home {
                Some(id) => {
                    tx.execute(
                        "INSERT OR IGNORE INTO members (community_id, user_id, joined_at) VALUES (?1, ?2, ?3)",
                        (&id, &user.id, now),
                    )?;
                    id
                }
                None => create_community(&tx, &user.id, "Paracord", &["general", "introductions"])?,
            };
            let token = create_session(&tx, &user.id)?;
            tx.commit()?;
            Ok((user, token, home))
        })
        .await?;

    let home_id = home.clone();
    let members = state.db(move |c| Ok(community_member_ids(c, &home_id)?)).await?;
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
    let wrong = || ApiError::BadRequest("That username and password don't match.".into());
    let username = normalize_username(&body.username).unwrap_or_default();

    let found: Option<(User, String)> = state
        .db(move |c| {
            Ok(c.query_row(
                "SELECT id, username, display_name, avatar_hue, password_hash FROM users WHERE username = ?1",
                [&username],
                |row| Ok((crate::auth::user_row(row, 0)?, row.get::<_, String>(4)?)),
            )
            .optional()?)
        })
        .await?;

    let password = body.password;
    let stored = found
        .as_ref()
        .map(|(_, hash)| hash.clone())
        .unwrap_or_else(|| dummy_hash().to_string());
    let ok = tokio::task::spawn_blocking(move || verify_password(&password, &stored))
        .await
        .map_err(ApiError::internal)?;
    let user = match found {
        Some((user, _)) if ok => user,
        _ => return Err(wrong()),
    };

    let user_id = user.id.clone();
    let token = state.db(move |c| Ok(create_session(c, &user_id)?)).await?;
    let cookie = session_cookie(&token, is_https(&state.cfg, &headers, &peer));
    Ok(([(SET_COOKIE, cookie)], Json(AuthResponse { user })).into_response())
}

pub async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if let Some(token) = session_token(&headers) {
        let hash = sha256_hex(&token);
        state
            .db(move |c| {
                c.execute("DELETE FROM sessions WHERE token_hash = ?1", [hash])?;
                Ok(())
            })
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
    let row = updated.clone();
    let audience = state
        .db(move |c| {
            c.execute(
                "UPDATE users SET display_name = ?1 WHERE id = ?2",
                (&row.display_name, &row.id),
            )?;
            Ok(related_user_ids(c, &row.id)?)
        })
        .await?;
    state.hub.send(&audience, &ServerEvent::UserUpdated { user: updated.clone() });
    Ok(Json(updated))
}
