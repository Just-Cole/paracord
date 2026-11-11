use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use paracord_shared::{
    Channel, Community, CreateChannel, DiscoverCommunity, MAX_TOPIC_LEN, Member, NameRequest,
    ServerEvent, UpdateChannel, channel_slug, clean_community_name,
};
use rusqlite::{Connection, OptionalExtension};

use crate::access::{
    channels_for, communities_for, community_member_ids, require_channel, require_member,
    require_owner,
};
use crate::auth::{CurrentUser, user_row};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{new_id, now_ms};

const MAX_CHANNELS: i64 = 100;

/// Creates a community owned by `owner_id`, with the owner as its first member.
pub fn create_community(
    c: &Connection,
    owner_id: &str,
    name: &str,
    channels: &[&str],
) -> rusqlite::Result<String> {
    let id = new_id();
    let now = now_ms();
    c.execute(
        "INSERT INTO communities (id, name, owner_id, created_at) VALUES (?1, ?2, ?3, ?4)",
        (&id, name, owner_id, now),
    )?;
    c.execute(
        "INSERT INTO members (community_id, user_id, joined_at) VALUES (?1, ?2, ?3)",
        (&id, owner_id, now),
    )?;
    for (position, channel) in channels.iter().enumerate() {
        c.execute(
            "INSERT INTO channels (id, community_id, name, topic, position, created_at)
             VALUES (?1, ?2, ?3, '', ?4, ?5)",
            (new_id(), &id, channel, position as i64, now),
        )?;
    }
    Ok(id)
}

fn tell_members(state: &AppState, members: &[String], event: ServerEvent) {
    state.hub.send(members, &event);
}

pub async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Vec<Community>>> {
    let list = state.db(move |c| Ok(communities_for(c, &user.id)?)).await?;
    Ok(Json(list))
}

pub async fn discover(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<DiscoverCommunity>>> {
    let list = state
        .db(move |c| {
            let mut stmt = c.prepare(
                "SELECT co.id, co.name, COUNT(m.user_id) AS n FROM communities co
                 LEFT JOIN members m ON m.community_id = co.id
                 WHERE co.id NOT IN (SELECT community_id FROM members WHERE user_id = ?1)
                 GROUP BY co.id ORDER BY n DESC, co.name LIMIT 50",
            )?;
            let rows = stmt
                .query_map([&user.id], |row| {
                    Ok(DiscoverCommunity {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        member_count: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(list))
}

pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<NameRequest>,
) -> ApiResult<(StatusCode, Json<Community>)> {
    if !state.allow(&format!("community:{}", user.id), 5, Duration::from_secs(60 * 60)) {
        return Err(ApiError::TooManyRequests);
    }
    let name = clean_community_name(&body.name).map_err(ApiError::bad)?;
    let user_id = user.id.clone();
    let community = state
        .db(move |c| {
            let tx = c.transaction()?;
            let id = create_community(&tx, &user_id, &name, &["general"])?;
            tx.commit()?;
            Ok(Community {
                channels: channels_for(c, &id)?,
                id,
                name,
                owner_id: user_id,
            })
        })
        .await?;
    state.hub.send_one(&user.id, &ServerEvent::CommunitiesChanged);
    Ok((StatusCode::CREATED, Json(community)))
}

pub async fn rename(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<NameRequest>,
) -> ApiResult<StatusCode> {
    let name = clean_community_name(&body.name).map_err(ApiError::bad)?;
    let members = state
        .db(move |c| {
            require_owner(c, &id, &user.id)?;
            c.execute("UPDATE communities SET name = ?1 WHERE id = ?2", (&name, &id))?;
            Ok(community_member_ids(c, &id)?)
        })
        .await?;
    tell_members(&state, &members, ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let members = state
        .db(move |c| {
            require_owner(c, &id, &user.id)?;
            let first: Option<String> = c
                .query_row("SELECT id FROM communities ORDER BY created_at, id LIMIT 1", [], |r| r.get(0))
                .optional()?;
            if first.as_deref() == Some(id.as_str()) {
                return Err(ApiError::forbidden(
                    "This is the home community everyone joins, so it can't be deleted.",
                ));
            }
            let members = community_member_ids(c, &id)?;
            c.execute("DELETE FROM communities WHERE id = ?1", [&id])?;
            Ok(members)
        })
        .await?;
    tell_members(&state, &members, ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn join(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let community_id = id.clone();
    let user_id = user.id.clone();
    let members = state
        .db(move |c| {
            let exists = c
                .query_row("SELECT 1 FROM communities WHERE id = ?1", [&id], |_| Ok(()))
                .optional()?
                .is_some();
            if !exists {
                return Err(ApiError::not_found("Community not found."));
            }
            c.execute(
                "INSERT OR IGNORE INTO members (community_id, user_id, joined_at) VALUES (?1, ?2, ?3)",
                (&id, &user_id, now_ms()),
            )?;
            Ok(community_member_ids(c, &id)?)
        })
        .await?;
    state.hub.send_one(&user.id, &ServerEvent::CommunitiesChanged);
    tell_members(&state, &members, ServerEvent::MembersChanged { community_id });
    Ok(StatusCode::NO_CONTENT)
}

pub async fn leave(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let community_id = id.clone();
    let user_id = user.id.clone();
    let members = state
        .db(move |c| {
            let owner = require_member(c, &id, &user_id)?;
            if owner == user_id {
                return Err(ApiError::forbidden("Owners can't leave their own community."));
            }
            c.execute(
                "DELETE FROM members WHERE community_id = ?1 AND user_id = ?2",
                (&id, &user_id),
            )?;
            Ok(community_member_ids(c, &id)?)
        })
        .await?;
    state.hub.send_one(&user.id, &ServerEvent::CommunitiesChanged);
    tell_members(&state, &members, ServerEvent::MembersChanged { community_id });
    Ok(StatusCode::NO_CONTENT)
}

pub async fn members(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<Member>>> {
    let users = state
        .db(move |c| {
            require_member(c, &id, &user.id)?;
            let mut stmt = c.prepare(
                "SELECT u.id, u.username, u.display_name, u.avatar_hue FROM members m
                 JOIN users u ON u.id = m.user_id WHERE m.community_id = ?1",
            )?;
            let users = stmt
                .query_map([&id], |row| user_row(row, 0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(users)
        })
        .await?;
    let online = state.hub.online_users();
    let mut members: Vec<Member> = users
        .into_iter()
        .map(|user| Member {
            online: online.contains(&user.id),
            user,
        })
        .collect();
    members.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| a.user.display_name.to_lowercase().cmp(&b.user.display_name.to_lowercase()))
    });
    Ok(Json(members))
}

fn clean_topic(raw: &str) -> ApiResult<String> {
    let topic = raw.trim();
    if topic.chars().count() > MAX_TOPIC_LEN {
        return Err(ApiError::bad("Topics can be at most 200 characters."));
    }
    Ok(topic.to_string())
}

pub async fn create_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<CreateChannel>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let name = channel_slug(&body.name).map_err(ApiError::bad)?;
    let topic = clean_topic(&body.topic)?;
    let (channel, members) = state
        .db(move |c| {
            require_owner(c, &id, &user.id)?;
            let (count, next): (i64, i64) = c.query_row(
                "SELECT COUNT(*), COALESCE(MAX(position), -1) + 1 FROM channels WHERE community_id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if count >= MAX_CHANNELS {
                return Err(ApiError::bad("A community can have at most 100 channels."));
            }
            let channel = Channel {
                id: new_id(),
                community_id: id.clone(),
                name,
                topic,
                position: next,
            };
            c.execute(
                "INSERT INTO channels (id, community_id, name, topic, position, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (&channel.id, &id, &channel.name, &channel.topic, next, now_ms()),
            )?;
            Ok((channel, community_member_ids(c, &id)?))
        })
        .await?;
    tell_members(&state, &members, ServerEvent::CommunitiesChanged);
    Ok((StatusCode::CREATED, Json(channel)))
}

pub async fn update_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateChannel>,
) -> ApiResult<StatusCode> {
    let name = body
        .name
        .as_deref()
        .map(channel_slug)
        .transpose()
        .map_err(ApiError::bad)?;
    let topic = body.topic.as_deref().map(clean_topic).transpose()?;
    let members = state
        .db(move |c| {
            let (community_id, owner) = require_channel(c, &id, &user.id)?;
            if owner != user.id {
                return Err(ApiError::forbidden("Only the community owner can do that."));
            }
            if let Some(name) = &name {
                c.execute("UPDATE channels SET name = ?1 WHERE id = ?2", (name, &id))?;
            }
            if let Some(topic) = &topic {
                c.execute("UPDATE channels SET topic = ?1 WHERE id = ?2", (topic, &id))?;
            }
            Ok(community_member_ids(c, &community_id)?)
        })
        .await?;
    tell_members(&state, &members, ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let members = state
        .db(move |c| {
            let (community_id, owner) = require_channel(c, &id, &user.id)?;
            if owner != user.id {
                return Err(ApiError::forbidden("Only the community owner can do that."));
            }
            let count: i64 = c.query_row(
                "SELECT COUNT(*) FROM channels WHERE community_id = ?1",
                [&community_id],
                |row| row.get(0),
            )?;
            if count <= 1 {
                return Err(ApiError::bad("A community needs at least one channel."));
            }
            c.execute("DELETE FROM channels WHERE id = ?1", [&id])?;
            Ok(community_member_ids(c, &community_id)?)
        })
        .await?;
    tell_members(&state, &members, ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}
