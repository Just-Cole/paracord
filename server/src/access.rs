//! Database lookups, including the ones that decide who may see or change what.

use paracord_shared::{Channel, Community, Message};
use sqlx::MySqlConnection;

use crate::auth::{USER_COLUMNS, UserRow, to_user};
use crate::error::{ApiError, ApiResult};

type Conn = MySqlConnection;

/// Returns the community's owner id, or "not found" if the user isn't in it
/// (outsiders can't tell a private community from a missing one).
pub async fn require_member(c: &mut Conn, community_id: &str, user_id: &str) -> ApiResult<String> {
    let owner: Option<String> = sqlx::query_scalar(
        "SELECT co.owner_id FROM communities co
         JOIN members m ON m.community_id = co.id AND m.user_id = ?
         WHERE co.id = ?",
    )
    .bind(user_id)
    .bind(community_id)
    .fetch_optional(&mut *c)
    .await?;
    owner.ok_or_else(|| ApiError::not_found("Community not found."))
}

pub async fn require_owner(c: &mut Conn, community_id: &str, user_id: &str) -> ApiResult<()> {
    let owner = require_member(c, community_id, user_id).await?;
    if owner != user_id {
        return Err(ApiError::forbidden("Only the community owner can do that."));
    }
    Ok(())
}

/// Returns (community id, owner id) for a channel the user can see.
pub async fn require_channel(c: &mut Conn, channel_id: &str, user_id: &str) -> ApiResult<(String, String)> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT ch.community_id, co.owner_id FROM channels ch
         JOIN communities co ON co.id = ch.community_id
         JOIN members m ON m.community_id = ch.community_id AND m.user_id = ?
         WHERE ch.id = ?",
    )
    .bind(user_id)
    .bind(channel_id)
    .fetch_optional(&mut *c)
    .await?;
    row.ok_or_else(|| ApiError::not_found("Channel not found."))
}

/// Returns the other participant of a conversation the user is in.
pub async fn require_conversation(c: &mut Conn, conversation_id: &str, user_id: &str) -> ApiResult<String> {
    let pair: Option<(String, String)> =
        sqlx::query_as("SELECT user_a, user_b FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_optional(&mut *c)
            .await?;
    match pair {
        Some((a, b)) if a == user_id => Ok(b),
        Some((a, b)) if b == user_id => Ok(a),
        _ => Err(ApiError::not_found("Conversation not found.")),
    }
}

pub async fn community_member_ids(c: &mut Conn, community_id: &str) -> ApiResult<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM members WHERE community_id = ?")
        .bind(community_id)
        .fetch_all(&mut *c)
        .await?)
}

/// Everyone who shares a community or a conversation with the user, plus the user.
pub async fn related_user_ids(c: &mut Conn, user_id: &str) -> ApiResult<Vec<String>> {
    let mut ids: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT m2.user_id FROM members m1
           JOIN members m2 ON m2.community_id = m1.community_id
           WHERE m1.user_id = ?
         UNION
         SELECT CASE WHEN user_a = ? THEN user_b ELSE user_a END
           FROM conversations WHERE user_a = ? OR user_b = ?",
    )
    .bind(user_id)
    .bind(user_id)
    .bind(user_id)
    .bind(user_id)
    .fetch_all(&mut *c)
    .await?;
    if !ids.iter().any(|id| id == user_id) {
        ids.push(user_id.to_string());
    }
    Ok(ids)
}

pub async fn shares_community(c: &mut Conn, a: &str, b: &str) -> ApiResult<bool> {
    let found: Option<String> = sqlx::query_scalar(
        "SELECT m1.community_id FROM members m1
         JOIN members m2 ON m2.community_id = m1.community_id
         WHERE m1.user_id = ? AND m2.user_id = ? LIMIT 1",
    )
    .bind(a)
    .bind(b)
    .fetch_optional(&mut *c)
    .await?;
    Ok(found.is_some())
}

pub async fn channels_for(c: &mut Conn, community_id: &str) -> ApiResult<Vec<Channel>> {
    let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, community_id, name, topic, position FROM channels
         WHERE community_id = ? ORDER BY position, id",
    )
    .bind(community_id)
    .fetch_all(&mut *c)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, community_id, name, topic, position)| Channel {
            id,
            community_id,
            name,
            topic,
            position,
        })
        .collect())
}

pub async fn communities_for(c: &mut Conn, user_id: &str) -> ApiResult<Vec<Community>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT co.id, co.name, co.owner_id FROM communities co
         JOIN members m ON m.community_id = co.id
         WHERE m.user_id = ? ORDER BY m.joined_at, co.id",
    )
    .bind(user_id)
    .fetch_all(&mut *c)
    .await?;
    let mut list = Vec::with_capacity(rows.len());
    for (id, name, owner_id) in rows {
        let channels = channels_for(c, &id).await?;
        list.push(Community {
            id,
            name,
            owner_id,
            channels,
        });
    }
    Ok(list)
}

/// The id of the first community ever created, which everyone joins.
pub async fn home_community(c: &mut Conn) -> ApiResult<Option<String>> {
    Ok(sqlx::query_scalar("SELECT id FROM communities ORDER BY created_at, id LIMIT 1")
        .fetch_optional(&mut *c)
        .await?)
}

type MessageRow = (
    String,
    Option<String>,
    Option<String>,
    String,
    i64,
    Option<i64>,
    String,
    String,
    String,
    i64,
);

fn message_select() -> String {
    format!(
        "SELECT m.id, m.channel_id, m.conversation_id, m.content, m.created_at, m.edited_at, {USER_COLUMNS}
         FROM messages m JOIN users u ON u.id = m.author_id"
    )
}

fn to_message(row: MessageRow) -> Message {
    let (id, channel_id, conversation_id, content, created_at, edited_at, uid, username, display_name, hue) = row;
    Message {
        id,
        channel_id,
        conversation_id,
        content,
        created_at,
        edited_at,
        author: to_user((uid, username, display_name, hue) as UserRow),
    }
}

pub async fn get_message(c: &mut Conn, id: &str) -> ApiResult<Option<Message>> {
    let row: Option<MessageRow> = sqlx::query_as(sql!("{} WHERE m.id = ?", message_select()))
        .bind(id)
        .fetch_optional(&mut *c)
        .await?;
    Ok(row.map(to_message))
}

pub enum Scope<'a> {
    Channel(&'a str),
    Conversation(&'a str),
}

/// Newest page of messages (oldest first), optionally before a message id.
pub async fn list_messages(
    c: &mut Conn,
    scope: Scope<'_>,
    before: Option<&str>,
    limit: usize,
) -> ApiResult<Vec<Message>> {
    let (column, id) = match scope {
        Scope::Channel(id) => ("m.channel_id", id),
        Scope::Conversation(id) => ("m.conversation_id", id),
    };
    // Ids sort by creation time, so "before" is a plain text comparison.
    let sql = format!(
        "{} WHERE {column} = ? AND (? IS NULL OR m.id < ?) ORDER BY m.id DESC LIMIT ?",
        message_select()
    );
    let rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .bind(before)
        .bind(before)
        .bind(limit as i64)
        .fetch_all(&mut *c)
        .await?;
    let mut messages: Vec<Message> = rows.into_iter().map(to_message).collect();
    messages.reverse();
    Ok(messages)
}
