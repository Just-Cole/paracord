use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use paracord_shared::{
    ContentRequest, Conversation, MESSAGE_PAGE, Message, OpenDm, ServerEvent, User, clean_message,
    normalize_username,
};
use rusqlite::{Connection, OptionalExtension};

use super::messages::{Page, insert_message, limit_messages};
use crate::access::{Scope, list_messages as fetch_messages, require_conversation, shares_community};
use crate::auth::{CurrentUser, user_row};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{new_id, now_ms};

fn conversations_for(c: &Connection, user_id: &str) -> rusqlite::Result<Vec<Conversation>> {
    let mut stmt = c.prepare_cached(
        "SELECT cv.id, cv.last_message_at, u.id, u.username, u.display_name, u.avatar_hue
         FROM conversations cv
         JOIN users u ON u.id = CASE WHEN cv.user_a = ?1 THEN cv.user_b ELSE cv.user_a END
         WHERE cv.user_a = ?1 OR cv.user_b = ?1
         ORDER BY cv.last_message_at DESC",
    )?;
    stmt.query_map([user_id], |row| {
        Ok(Conversation {
            id: row.get(0)?,
            last_message_at: row.get(1)?,
            user: user_row(row, 2)?,
        })
    })?
    .collect()
}

pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Conversation>>> {
    let list = state.db(move |c| Ok(conversations_for(c, &user.id)?)).await?;
    Ok(Json(list))
}

/// Opens (or reuses) a direct conversation with someone you share a community with.
pub async fn open(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<OpenDm>,
) -> ApiResult<Json<Conversation>> {
    let not_found = || ApiError::not_found("No one you share a community with has that username.");
    let username = normalize_username(&body.username).map_err(|_| not_found())?;
    let me = user.id.clone();
    let (conversation, created) = state
        .db(move |c| {
            let other: Option<User> = c
                .query_row(
                    "SELECT id, username, display_name, avatar_hue FROM users WHERE username = ?1",
                    [&username],
                    |row| user_row(row, 0),
                )
                .optional()?;
            let other = other.ok_or_else(not_found)?;
            if other.id == me {
                return Err(ApiError::bad("You can't message yourself."));
            }
            let (a, b) = if me < other.id { (me.as_str(), other.id.as_str()) } else { (other.id.as_str(), me.as_str()) };
            let existing: Option<(String, i64)> = c
                .query_row(
                    "SELECT id, last_message_at FROM conversations WHERE user_a = ?1 AND user_b = ?2",
                    (a, b),
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((id, last_message_at)) = existing {
                return Ok((Conversation { id, user: other, last_message_at }, false));
            }
            if !shares_community(c, &me, &other.id)? {
                return Err(not_found());
            }
            let id = new_id();
            let now = now_ms();
            c.execute(
                "INSERT INTO conversations (id, user_a, user_b, created_at, last_message_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                (&id, a, b, now),
            )?;
            Ok((Conversation { id, user: other, last_message_at: now }, true))
        })
        .await?;
    if created {
        state.hub.send([&user.id, &conversation.user.id], &ServerEvent::DmsChanged);
    }
    Ok(Json(conversation))
}

pub async fn list_messages(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<Json<Vec<Message>>> {
    let messages = state
        .db(move |c| {
            require_conversation(c, &id, &user.id)?;
            Ok(fetch_messages(c, Scope::Conversation(&id), page.before.as_deref(), MESSAGE_PAGE)?)
        })
        .await?;
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
    let me = user.id.clone();
    let (message, other) = state
        .db(move |c| {
            let other = require_conversation(c, &id, &me)?;
            let message = insert_message(c, Scope::Conversation(&id), &me, &content)?;
            c.execute(
                "UPDATE conversations SET last_message_at = ?1 WHERE id = ?2",
                (message.created_at, &id),
            )?;
            Ok((message, other))
        })
        .await?;
    state.hub.send(
        [&user.id, &other],
        &ServerEvent::MessageCreated { message: message.clone() },
    );
    Ok((StatusCode::CREATED, Json(message)))
}
