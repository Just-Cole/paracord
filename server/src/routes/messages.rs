use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use paracord_shared::{ContentRequest, MESSAGE_PAGE, Message, ServerEvent, clean_message};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;

use crate::access::{
    Scope, community_member_ids, get_message, list_messages, require_channel, require_conversation,
};
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{new_id, now_ms};

#[derive(Deserialize)]
pub struct Page {
    pub before: Option<String>,
}

pub fn limit_messages(state: &AppState, user_id: &str) -> ApiResult<()> {
    if !state.allow(&format!("msg:{user_id}"), 20, Duration::from_secs(10)) {
        return Err(ApiError::TooManyRequests);
    }
    Ok(())
}

/// Inserts a message and returns it with its author.
pub fn insert_message(
    c: &Connection,
    scope: Scope<'_>,
    author_id: &str,
    content: &str,
) -> ApiResult<Message> {
    let id = new_id();
    let (channel_id, conversation_id) = match scope {
        Scope::Channel(id) => (Some(id), None),
        Scope::Conversation(id) => (None, Some(id)),
    };
    c.execute(
        "INSERT INTO messages (id, channel_id, conversation_id, author_id, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (&id, channel_id, conversation_id, author_id, content, now_ms()),
    )?;
    get_message(c, &id)?.ok_or_else(|| ApiError::internal("message vanished after insert"))
}

pub async fn list_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<Json<Vec<Message>>> {
    let messages = state
        .db(move |c| {
            require_channel(c, &id, &user.id)?;
            Ok(list_messages(c, Scope::Channel(&id), page.before.as_deref(), MESSAGE_PAGE)?)
        })
        .await?;
    Ok(Json(messages))
}

pub async fn send_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<ContentRequest>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    limit_messages(&state, &user.id)?;
    let content = clean_message(&body.content).map_err(ApiError::bad)?;
    let (message, audience) = state
        .db(move |c| {
            let (community_id, _) = require_channel(c, &id, &user.id)?;
            let message = insert_message(c, Scope::Channel(&id), &user.id, &content)?;
            Ok((message, community_member_ids(c, &community_id)?))
        })
        .await?;
    state.hub.send(&audience, &ServerEvent::MessageCreated { message: message.clone() });
    Ok((StatusCode::CREATED, Json(message)))
}

struct Target {
    author_id: String,
    /// Who should hear about changes to this message.
    audience: Vec<String>,
    /// Whether the acting user owns the community the message is in.
    actor_owns_community: bool,
}

/// Loads a message the user can see, with who else can see it.
fn load_target(c: &Connection, message_id: &str, user_id: &str) -> ApiResult<Target> {
    let row: Option<(String, Option<String>, Option<String>)> = c
        .query_row(
            "SELECT author_id, channel_id, conversation_id FROM messages WHERE id = ?1",
            [message_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let not_found = || ApiError::not_found("Message not found.");
    let (author_id, channel_id, conversation_id) = row.ok_or_else(not_found)?;
    if let Some(channel_id) = channel_id {
        let (community_id, owner) = require_channel(c, &channel_id, user_id).map_err(|_| not_found())?;
        return Ok(Target {
            author_id,
            audience: community_member_ids(c, &community_id)?,
            actor_owns_community: owner == user_id,
        });
    }
    let conversation_id = conversation_id.ok_or_else(not_found)?;
    let other = require_conversation(c, &conversation_id, user_id).map_err(|_| not_found())?;
    Ok(Target {
        author_id,
        audience: vec![user_id.to_string(), other],
        actor_owns_community: false,
    })
}

pub async fn edit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<ContentRequest>,
) -> ApiResult<Json<Message>> {
    limit_messages(&state, &user.id)?;
    let content = clean_message(&body.content).map_err(ApiError::bad)?;
    let (message, audience) = state
        .db(move |c| {
            let target = load_target(c, &id, &user.id)?;
            if target.author_id != user.id {
                return Err(ApiError::forbidden("You can only edit your own messages."));
            }
            c.execute(
                "UPDATE messages SET content = ?1, edited_at = ?2 WHERE id = ?3",
                (&content, now_ms(), &id),
            )?;
            let message = get_message(c, &id)?.ok_or_else(|| ApiError::not_found("Message not found."))?;
            Ok((message, target.audience))
        })
        .await?;
    state.hub.send(&audience, &ServerEvent::MessageUpdated { message: message.clone() });
    Ok(Json(message))
}

pub async fn delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let (event, audience) = state
        .db(move |c| {
            let target = load_target(c, &id, &user.id)?;
            if target.author_id != user.id && !target.actor_owns_community {
                return Err(ApiError::forbidden("You can only delete your own messages."));
            }
            let (channel_id, conversation_id): (Option<String>, Option<String>) = c.query_row(
                "SELECT channel_id, conversation_id FROM messages WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            c.execute("DELETE FROM messages WHERE id = ?1", [&id])?;
            Ok((
                ServerEvent::MessageDeleted {
                    id,
                    channel_id,
                    conversation_id,
                },
                target.audience,
            ))
        })
        .await?;
    state.hub.send(&audience, &event);
    Ok(StatusCode::NO_CONTENT)
}
