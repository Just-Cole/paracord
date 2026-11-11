use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use paracord_shared::{
    ContentRequest, Conversation, MESSAGE_PAGE, Message, OpenDm, ServerEvent, clean_message,
    normalize_username,
};

use super::messages::{Page, insert_message, limit_messages};
use crate::access::{Scope, list_messages as fetch_messages, require_conversation, shares_community};
use crate::auth::{CurrentUser, USER_COLUMNS, UserRow, to_user};
use crate::error::{ApiError, ApiResult, is_duplicate};
use crate::state::AppState;
use crate::util::{new_id, now_ms};

pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Conversation>>> {
    let rows: Vec<(String, i64, String, String, String, i64)> = sqlx::query_as(sql!(
        "SELECT cv.id, cv.last_message_at, {USER_COLUMNS}
         FROM conversations cv
         JOIN users u ON u.id = CASE WHEN cv.user_a = ? THEN cv.user_b ELSE cv.user_a END
         WHERE cv.user_a = ? OR cv.user_b = ?
         ORDER BY cv.last_message_at DESC"
    ))
    .bind(&user.id)
    .bind(&user.id)
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, last_message_at, uid, username, display_name, hue)| Conversation {
                id,
                last_message_at,
                user: to_user((uid, username, display_name, hue)),
            })
            .collect(),
    ))
}

/// Opens (or reuses) a direct conversation with someone you share a community with.
pub async fn open(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<OpenDm>,
) -> ApiResult<Json<Conversation>> {
    let not_found = || ApiError::not_found("No one you share a community with has that username.");
    let username = normalize_username(&body.username).map_err(|_| not_found())?;
    let mut c = state.conn().await?;
    let other: Option<UserRow> = sqlx::query_as(sql!("SELECT {USER_COLUMNS} FROM users u WHERE u.username = ?"))
        .bind(&username)
        .fetch_optional(&mut *c)
        .await?;
    let other = to_user(other.ok_or_else(not_found)?);
    if other.id == user.id {
        return Err(ApiError::bad("You can't message yourself."));
    }
    let (a, b) = if user.id < other.id { (&user.id, &other.id) } else { (&other.id, &user.id) };

    let find = async |c: &mut sqlx::MySqlConnection| -> ApiResult<Option<(String, i64)>> {
        Ok(sqlx::query_as("SELECT id, last_message_at FROM conversations WHERE user_a = ? AND user_b = ?")
            .bind(a)
            .bind(b)
            .fetch_optional(&mut *c)
            .await?)
    };
    if let Some((id, last_message_at)) = find(&mut c).await? {
        return Ok(Json(Conversation { id, user: other, last_message_at }));
    }
    if !shares_community(&mut c, &user.id, &other.id).await? {
        return Err(not_found());
    }
    let id = new_id();
    let now = now_ms();
    let inserted = sqlx::query(
        "INSERT INTO conversations (id, user_a, user_b, created_at, last_message_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(a)
    .bind(b)
    .bind(now)
    .bind(now)
    .execute(&mut *c)
    .await;
    match inserted {
        Ok(_) => {}
        // Both people opened it at the same moment: use the one that won.
        Err(error) if is_duplicate(&error) => {
            if let Some((id, last_message_at)) = find(&mut c).await? {
                return Ok(Json(Conversation { id, user: other, last_message_at }));
            }
            return Err(error.into());
        }
        Err(error) => return Err(error.into()),
    }
    state.hub.send([&user.id, &other.id], &ServerEvent::DmsChanged);
    Ok(Json(Conversation { id, user: other, last_message_at: now }))
}

pub async fn list_messages(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<Json<Vec<Message>>> {
    let mut c = state.conn().await?;
    require_conversation(&mut c, &id, &user.id).await?;
    let messages =
        fetch_messages(&mut c, Scope::Conversation(&id), page.before.as_deref(), MESSAGE_PAGE).await?;
    Ok(Json(messages))
}

pub async fn send(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<ContentRequest>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    limit_messages(&state, &user.id)?;
    let content = clean_message(&body.content).map_err(ApiError::bad)?;
    let mut c = state.conn().await?;
    let other = require_conversation(&mut c, &id, &user.id).await?;
    let message = insert_message(&mut c, Scope::Conversation(&id), &user.id, &content).await?;
    sqlx::query("UPDATE conversations SET last_message_at = ? WHERE id = ?")
        .bind(message.created_at)
        .bind(&id)
        .execute(&mut *c)
        .await?;
    state.hub.send(
        [&user.id, &other],
        &ServerEvent::MessageCreated { message: message.clone() },
    );
    Ok((StatusCode::CREATED, Json(message)))
}
