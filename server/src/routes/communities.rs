use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use paracord_shared::{
    Channel, Community, CreateChannel, DiscoverCommunity, MAX_TOPIC_LEN, Member, NameRequest,
    ServerEvent, UpdateChannel, channel_slug, clean_community_name,
};
use sqlx::MySqlConnection;

use crate::access::{
    channels_for, communities_for, community_member_ids, home_community, require_channel,
    require_member, require_owner,
};
use crate::auth::{CurrentUser, USER_COLUMNS, UserRow, to_user};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{new_id, now_ms};

const MAX_CHANNELS: i64 = 100;

/// Creates a community owned by `owner_id`, with the owner as its first member.
pub async fn create_community(
    c: &mut MySqlConnection,
    owner_id: &str,
    name: &str,
    channels: &[&str],
) -> ApiResult<String> {
    let id = new_id();
    let now = now_ms();
    sqlx::query("INSERT INTO communities (id, name, owner_id, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(owner_id)
        .bind(now)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO members (community_id, user_id, joined_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(owner_id)
        .bind(now)
        .execute(&mut *c)
        .await?;
    for (position, channel) in channels.iter().enumerate() {
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, topic, position, created_at)
             VALUES (?, ?, ?, '', ?, ?)",
        )
        .bind(new_id())
        .bind(&id)
        .bind(channel)
        .bind(position as i64)
        .bind(now)
        .execute(&mut *c)
        .await?;
    }
    Ok(id)
}

pub async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Vec<Community>>> {
    let mut c = state.conn().await?;
    Ok(Json(communities_for(&mut c, &user.id).await?))
}

pub async fn discover(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<DiscoverCommunity>>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT co.id, co.name, COUNT(m.user_id) AS n FROM communities co
         LEFT JOIN members m ON m.community_id = co.id
         WHERE co.id NOT IN (SELECT community_id FROM members WHERE user_id = ?)
         GROUP BY co.id, co.name ORDER BY n DESC, co.name LIMIT 50",
    )
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, member_count)| DiscoverCommunity { id, name, member_count })
            .collect(),
    ))
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
    let mut tx = state.pool.begin().await?;
    let id = create_community(&mut tx, &user.id, &name, &["general"]).await?;
    let channels = channels_for(&mut tx, &id).await?;
    tx.commit().await?;
    state.hub.send_one(&user.id, &ServerEvent::CommunitiesChanged);
    Ok((
        StatusCode::CREATED,
        Json(Community {
            id,
            name,
            owner_id: user.id,
            channels,
        }),
    ))
}

pub async fn rename(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<NameRequest>,
) -> ApiResult<StatusCode> {
    let name = clean_community_name(&body.name).map_err(ApiError::bad)?;
    let mut c = state.conn().await?;
    require_owner(&mut c, &id, &user.id).await?;
    sqlx::query("UPDATE communities SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(&id)
        .execute(&mut *c)
        .await?;
    let members = community_member_ids(&mut c, &id).await?;
    state.hub.send(&members, &ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut c = state.conn().await?;
    require_owner(&mut c, &id, &user.id).await?;
    if home_community(&mut c).await?.as_deref() == Some(id.as_str()) {
        return Err(ApiError::forbidden(
            "This is the home community everyone joins, so it can't be deleted.",
        ));
    }
    let members = community_member_ids(&mut c, &id).await?;
    sqlx::query("DELETE FROM communities WHERE id = ?")
        .bind(&id)
        .execute(&mut *c)
        .await?;
    state.hub.send(&members, &ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn join(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut c = state.conn().await?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM communities WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *c)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("Community not found."));
    }
    sqlx::query("INSERT IGNORE INTO members (community_id, user_id, joined_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&user.id)
        .bind(now_ms())
        .execute(&mut *c)
        .await?;
    let members = community_member_ids(&mut c, &id).await?;
    state.hub.send_one(&user.id, &ServerEvent::CommunitiesChanged);
    state.hub.send(&members, &ServerEvent::MembersChanged { community_id: id });
    Ok(StatusCode::NO_CONTENT)
}

pub async fn leave(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut c = state.conn().await?;
    let owner = require_member(&mut c, &id, &user.id).await?;
    if owner == user.id {
        return Err(ApiError::forbidden("Owners can't leave their own community."));
    }
    sqlx::query("DELETE FROM members WHERE community_id = ? AND user_id = ?")
        .bind(&id)
        .bind(&user.id)
        .execute(&mut *c)
        .await?;
    let members = community_member_ids(&mut c, &id).await?;
    state.hub.send_one(&user.id, &ServerEvent::CommunitiesChanged);
    state.hub.send(&members, &ServerEvent::MembersChanged { community_id: id });
    Ok(StatusCode::NO_CONTENT)
}

pub async fn members(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<Member>>> {
    let mut c = state.conn().await?;
    require_member(&mut c, &id, &user.id).await?;
    let rows: Vec<UserRow> = sqlx::query_as(sql!(
        "SELECT {USER_COLUMNS} FROM members m JOIN users u ON u.id = m.user_id WHERE m.community_id = ?"
    ))
    .bind(&id)
    .fetch_all(&mut *c)
    .await?;
    let online = state.hub.online_users();
    let mut members: Vec<Member> = rows
        .into_iter()
        .map(to_user)
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
    let mut c = state.conn().await?;
    require_owner(&mut c, &id, &user.id).await?;
    let (count, next): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), CAST(COALESCE(MAX(position), -1) + 1 AS SIGNED) FROM channels WHERE community_id = ?",
    )
    .bind(&id)
    .fetch_one(&mut *c)
    .await?;
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
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, topic, position, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&channel.id)
    .bind(&id)
    .bind(&channel.name)
    .bind(&channel.topic)
    .bind(next)
    .bind(now_ms())
    .execute(&mut *c)
    .await?;
    let members = community_member_ids(&mut c, &id).await?;
    state.hub.send(&members, &ServerEvent::CommunitiesChanged);
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
    let mut c = state.conn().await?;
    let (community_id, owner) = require_channel(&mut c, &id, &user.id).await?;
    if owner != user.id {
        return Err(ApiError::forbidden("Only the community owner can do that."));
    }
    if let Some(name) = &name {
        sqlx::query("UPDATE channels SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&id)
            .execute(&mut *c)
            .await?;
    }
    if let Some(topic) = &topic {
        sqlx::query("UPDATE channels SET topic = ? WHERE id = ?")
            .bind(topic)
            .bind(&id)
            .execute(&mut *c)
            .await?;
    }
    let members = community_member_ids(&mut c, &community_id).await?;
    state.hub.send(&members, &ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_channel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut c = state.conn().await?;
    let (community_id, owner) = require_channel(&mut c, &id, &user.id).await?;
    if owner != user.id {
        return Err(ApiError::forbidden("Only the community owner can do that."));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE community_id = ?")
        .bind(&community_id)
        .fetch_one(&mut *c)
        .await?;
    if count <= 1 {
        return Err(ApiError::bad("A community needs at least one channel."));
    }
    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(&id)
        .execute(&mut *c)
        .await?;
    let members = community_member_ids(&mut c, &community_id).await?;
    state.hub.send(&members, &ServerEvent::CommunitiesChanged);
    Ok(StatusCode::NO_CONTENT)
}
