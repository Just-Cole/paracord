//! Everything the server and the browser both need to agree on: the shapes of
//! API requests and responses, real-time events, and input rules.

use serde::{Deserialize, Serialize};

pub const MAX_MESSAGE_LEN: usize = 2000;
pub const MAX_TOPIC_LEN: usize = 200;
pub const MESSAGE_PAGE: usize = 50;

// ---------- Data ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub avatar_hue: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub community_id: String,
    pub name: String,
    pub topic: String,
    pub position: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Community {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub channels: Vec<Channel>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverCommunity {
    pub id: String,
    pub name: String,
    pub member_count: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub user: User,
    pub online: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub user: User,
    pub last_message_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub channel_id: Option<String>,
    pub conversation_id: Option<String>,
    pub author: User,
    pub content: String,
    pub created_at: i64,
    pub edited_at: Option<i64>,
}

// ---------- Requests and responses ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub user: User,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMe {
    pub display_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NameRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannel {
    pub name: String,
    #[serde(default)]
    pub topic: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ContentRequest {
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenDm {
    pub username: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ErrorBody {
    pub error: String,
}

// ---------- Real-time events ----------

/// Sent by the server over the WebSocket.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerEvent {
    Hello { user_id: String },
    Pong,
    MessageCreated { message: Message },
    MessageUpdated { message: Message },
    MessageDeleted {
        id: String,
        channel_id: Option<String>,
        conversation_id: Option<String>,
    },
    /// Your communities or their channels changed; fetch the list again.
    CommunitiesChanged,
    MembersChanged { community_id: String },
    DmsChanged,
    Presence { user_id: String, online: bool },
    UserUpdated { user: User },
    Typing {
        user: User,
        channel_id: Option<String>,
        conversation_id: Option<String>,
    },
}

/// Sent by the browser over the WebSocket.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClientEvent {
    Ping,
    Typing {
        #[serde(default)]
        channel_id: Option<String>,
        #[serde(default)]
        conversation_id: Option<String>,
    },
}

// ---------- Input rules (checked in the browser and enforced on the server) ----------

/// Lowercases and checks a username: 3-24 of a-z, 0-9, `_` and `.`.
pub fn normalize_username(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim().to_lowercase();
    let len = name.chars().count();
    if !(3..=24).contains(&len) {
        return Err("Usernames are 3 to 24 characters.");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
    {
        return Err("Usernames can use letters, numbers, _ and . only.");
    }
    Ok(name)
}

pub fn check_password(password: &str) -> Result<(), &'static str> {
    let len = password.chars().count();
    if len < 8 {
        return Err("Passwords need at least 8 characters.");
    }
    if len > 128 {
        return Err("Passwords can be at most 128 characters.");
    }
    Ok(())
}

pub fn clean_display_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    let len = name.chars().count();
    if !(1..=32).contains(&len) {
        return Err("Display names are 1 to 32 characters.");
    }
    Ok(name.to_string())
}

pub fn clean_community_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    let len = name.chars().count();
    if !(2..=40).contains(&len) {
        return Err("Community names are 2 to 40 characters.");
    }
    Ok(name.to_string())
}

/// Turns "Game Night!" into "game-night".
pub fn channel_slug(raw: &str) -> Result<String, &'static str> {
    let mut slug = String::new();
    for c in raw.trim().to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' {
            slug.push(c);
        } else if (c == '-' || c.is_whitespace()) && !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(32).collect();
    if slug.is_empty() {
        return Err("Channel names need at least one letter or number.");
    }
    Ok(slug)
}

pub fn clean_message(raw: &str) -> Result<String, &'static str> {
    let content = raw.trim();
    if content.is_empty() {
        return Err("Messages can't be empty.");
    }
    if content.chars().count() > MAX_MESSAGE_LEN {
        return Err("Messages can be at most 2000 characters.");
    }
    Ok(content.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(channel_slug("  Game Night! ").unwrap(), "game-night");
        assert_eq!(channel_slug("a -- b").unwrap(), "a-b");
        assert!(channel_slug("!!!").is_err());
    }

    #[test]
    fn usernames() {
        assert_eq!(normalize_username(" Ty.J ").unwrap(), "ty.j");
        assert!(normalize_username("ab").is_err());
        assert!(normalize_username("bad name").is_err());
    }

    #[test]
    fn events_round_trip() {
        let json = r#"{"type":"typing","channelId":"c1"}"#;
        let event: ClientEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, ClientEvent::Typing { channel_id: Some(_), conversation_id: None }));
    }
}
