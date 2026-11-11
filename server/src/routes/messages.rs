use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use paracord_shared::{ContentRequest, MESSAGE_PAGE, Message, ServerEvent, clean_message};
use serde::Deserialize;
use sqlx::MySqlConnection;

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
pub async fn insert_message(
    c: &mut MySqlConnection,
    scope: Scope<'_>,
    author_id: &str,
    content: &str,
) -> ApiResult<Message> {
    let id = new_id();
    let (channel_id, conversation_id) = match scope {
        Scope::Channel(id) => (Some(id), None),
        Scope::Conversation(id) => (None, Some(id)),
    };
    sqlx::query(
        "INSERT INTO messages (id, channel_id, conversation_id, author_id, content, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(channel_id)
    .bind(conversation_id)
    .bind(author_id)
    .bind(content)
    .bind(now_ms())
    .execute(&mut *c)
    .await?;
    get_message(c, &id)
        .await?
        .ok_or_else(|| ApiError::internal("message vanished after insert"))
}

pub async fn list_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<Json<Vec<Message>>> {
    let mut c = state.conn().await?;
    require_channel(&mut c, &id, &user.id).await?;
    let messages = list_messages(&mut c, Scope::Channel(&id), page.before.as_deref(), MESSAGE_PAGE).await?;
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
    let mut c = state.conn().await?;
    let (community_id, _) = require_channel(&mut c, &id, &user.id).await?;
    let message = insert_message(&mut c, Scope::Channel(&id), &user.id, &content).await?;
    let audience = community_member_ids(&mut c, &community_id).await?;
    state.hub.send(&audience, &ServerEvent::MessageCreated { message: message.clone() });
    Ok((StatusCode::CREATED, Json(message)))
}

struct Target {
    author_id: String,
    channel_id: Option<String>,
    conversation_id: Option<String>,
    /// Who should hear about changes to this message.
    audience: Vec<String>,
    /// Whether the acting user owns the community the message is in.
    actor_owns_community: bool,
}

/// Loads a message the user can see, with who else can see it.
async fn load_target(c: &mut MySqlConnection, message_id: &str, user_id: &str) -> ApiResult<Target> {
    let row: Option<(String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT author_id, channel_id, conversation_id FROM messages WHERE id = ?")
            .bind(message_id)
            .fetch_optional(&mut *c)
            .await?;
    let not_found = || ApiError::not_found("Message not found.");
    let (author_id, channel_id, conversation_id) = row.ok_or_else(not_found)?;
    if let Some(channel) = &channel_id {
        let (community_id, owner) = require_channel(c, channel, user_id).await.map_err(|_| not_found())?;
        return Ok(Target {
            author_id,
            channel_id,
            conversation_id,
            audience: community_member_ids(c, &community_id).await?,
            actor_owns_community: owner == user_id,
        });
    }
    let conversation = conversation_id.clone().ok_or_else(not_found)?;
    let other = require_conversation(c, &conversation, user_id).await.map_err(|_| not_found())?;
    Ok(Target {
        author_id,
        channel_id,
        conversation_id,
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
    let mut c = state.conn().await?;
    let target = load_target(&mut c, &id, &user.id).await?;
    if target.author_id != user.id {
        return Err(ApiError::forbidden("You can only edit your own messages."));
    }
    sqlx::query("UPDATE messages SET content = ?, edited_at = ? WHERE id = ?")
        .bind(&content)
        .bind(now_ms())
        .bind(&id)
        .execute(&mut *c)
        .await?;
    let message = get_message(&mut c, &id)
        .await?
        .ok_or_else(|| ApiError::not_found("Message not found."))?;
    state.hub.send(&target.audience, &ServerEvent::MessageUpdated { message: message.clone() });
    Ok(Json(message))
}

pub async fn delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut c = state.conn().await?;
    let target = load_target(&mut c, &id, &user.id).await?;
    if target.author_id != user.id && !target.actor_owns_community {
        return Err(ApiError::forbidden("You can only delete your own messages."));
    }
    sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(&id)
        .execute(&mut *c)
        .await?;
    state.hub.send(
        &target.audience,
        &ServerEvent::MessageDeleted {
            id,
            channel_id: target.channel_id,
            conversation_id: target.conversation_id,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}
