//! Lookups that decide who may see or change what.

use paracord_shared::{Channel, Community, Message};
use rusqlite::{Connection, OptionalExtension};

use crate::auth::user_row;
use crate::error::{ApiError, ApiResult};

/// Returns the community's owner id, or "not found" if the user isn't in it
/// (outsiders can't tell a private community from a missing one).
pub fn require_member(c: &Connection, community_id: &str, user_id: &str) -> ApiResult<String> {
    let owner: Option<String> = c
        .query_row(
            "SELECT co.owner_id FROM communities co
             JOIN members m ON m.community_id = co.id AND m.user_id = ?2
             WHERE co.id = ?1",
            (community_id, user_id),
            |row| row.get(0),
        )
        .optional()?;
    owner.ok_or_else(|| ApiError::not_found("Community not found."))
}

pub fn require_owner(c: &Connection, community_id: &str, user_id: &str) -> ApiResult<()> {
    let owner = require_member(c, community_id, user_id)?;
    if owner != user_id {
        return Err(ApiError::forbidden("Only the community owner can do that."));
    }
    Ok(())
}

/// Returns (community id, owner id) for a channel the user can see.
pub fn require_channel(c: &Connection, channel_id: &str, user_id: &str) -> ApiResult<(String, String)> {
    let community_id: Option<String> = c
        .query_row(
            "SELECT community_id FROM channels WHERE id = ?1",
            [channel_id],
            |row| row.get(0),
        )
        .optional()?;
    let community_id = community_id.ok_or_else(|| ApiError::not_found("Channel not found."))?;
    let owner = require_member(c, &community_id, user_id)
        .map_err(|_| ApiError::not_found("Channel not found."))?;
    Ok((community_id, owner))
}

/// Returns the other participant of a conversation the user is in.
pub fn require_conversation(c: &Connection, conversation_id: &str, user_id: &str) -> ApiResult<String> {
    let pair: Option<(String, String)> = c
        .query_row(
            "SELECT user_a, user_b FROM conversations WHERE id = ?1",
            [conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match pair {
        Some((a, b)) if a == user_id => Ok(b),
        Some((a, b)) if b == user_id => Ok(a),
        _ => Err(ApiError::not_found("Conversation not found.")),
    }
}

pub fn community_member_ids(c: &Connection, community_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare_cached("SELECT user_id FROM members WHERE community_id = ?1")?;
    stmt.query_map([community_id], |row| row.get(0))?.collect()
}

/// Everyone who shares a community or a conversation with the user, plus the user.
pub fn related_user_ids(c: &Connection, user_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare_cached(
        "SELECT DISTINCT m2.user_id FROM members m1
           JOIN members m2 ON m2.community_id = m1.community_id
           WHERE m1.user_id = ?1
         UNION
         SELECT CASE WHEN user_a = ?1 THEN user_b ELSE user_a END
           FROM conversations WHERE user_a = ?1 OR user_b = ?1
         UNION SELECT ?1",
    )?;
    stmt.query_map([user_id], |row| row.get(0))?.collect()
}

pub fn shares_community(c: &Connection, a: &str, b: &str) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT 1 FROM members m1 JOIN members m2 ON m2.community_id = m1.community_id
         WHERE m1.user_id = ?1 AND m2.user_id = ?2 LIMIT 1",
        (a, b),
        |_| Ok(()),
    )
    .optional()
    .map(|r| r.is_some())
}

pub fn channels_for(c: &Connection, community_id: &str) -> rusqlite::Result<Vec<Channel>> {
    let mut stmt = c.prepare_cached(
        "SELECT id, community_id, name, topic, position FROM channels
         WHERE community_id = ?1 ORDER BY position, id",
    )?;
    stmt.query_map([community_id], |row| {
        Ok(Channel {
            id: row.get(0)?,
            community_id: row.get(1)?,
            name: row.get(2)?,
            topic: row.get(3)?,
            position: row.get(4)?,
        })
    })?
    .collect()
}

pub fn communities_for(c: &Connection, user_id: &str) -> rusqlite::Result<Vec<Community>> {
    let mut stmt = c.prepare_cached(
        "SELECT co.id, co.name, co.owner_id FROM communities co
         JOIN members m ON m.community_id = co.id
         WHERE m.user_id = ?1 ORDER BY m.joined_at, co.id",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([user_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, name, owner_id)| {
            let channels = channels_for(c, &id)?;
            Ok(Community {
                id,
                name,
                owner_id,
                channels,
            })
        })
        .collect()
}

const MESSAGE_SELECT: &str = "SELECT m.id, m.channel_id, m.conversation_id, m.content, m.created_at, m.edited_at,
            u.id, u.username, u.display_name, u.avatar_hue
     FROM messages m JOIN users u ON u.id = m.author_id";

fn message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        conversation_id: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
        edited_at: row.get(5)?,
        author: user_row(row, 6)?,
    })
}

pub fn get_message(c: &Connection, id: &str) -> rusqlite::Result<Option<Message>> {
    c.query_row(&format!("{MESSAGE_SELECT} WHERE m.id = ?1"), [id], message_row)
        .optional()
}

pub enum Scope<'a> {
    Channel(&'a str),
    Conversation(&'a str),
}

/// Newest page of messages (oldest first), optionally before a message id.
pub fn list_messages(
    c: &Connection,
    scope: Scope<'_>,
    before: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Message>> {
    let (column, id) = match scope {
        Scope::Channel(id) => ("m.channel_id", id),
        Scope::Conversation(id) => ("m.conversation_id", id),
    };
    // Ids sort by creation time, so "before" is a plain text comparison.
    let sql = format!(
        "{MESSAGE_SELECT} WHERE {column} = ?1 AND (?2 IS NULL OR m.id < ?2)
         ORDER BY m.id DESC LIMIT ?3"
    );
    let mut stmt = c.prepare_cached(&sql)?;
    let mut messages: Vec<Message> = stmt
        .query_map((id, before, limit as i64), message_row)?
        .collect::<rusqlite::Result<_>>()?;
    messages.reverse();
    Ok(messages)
}
