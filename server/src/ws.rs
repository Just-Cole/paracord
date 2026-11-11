//! The real-time connection. The browser opens one WebSocket after signing in;
//! the server pushes events (new messages, presence, typing) over it.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use paracord_shared::{ClientEvent, ServerEvent, User};
use tokio::sync::mpsc;
use tracing::warn;

use crate::access::{community_member_ids, related_user_ids, require_channel, require_conversation};
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::realtime::SOCKET_QUEUE;
use crate::routes::origin_allowed;
use crate::state::AppState;

const PING_EVERY: Duration = Duration::from_secs(25);
/// A socket that sends nothing (not even a pong) for this long is dropped.
const IDLE_LIMIT: Duration = Duration::from_secs(70);

pub async fn handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
    if !origin_allowed(&state, &headers) {
        return Err(ApiError::forbidden("Requests from other sites are not allowed."));
    }
    Ok(upgrade
        .max_message_size(16 * 1024)
        .on_upgrade(move |socket| run(state, user, socket)))
}

async fn run(state: AppState, user: User, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<String>(SOCKET_QUEUE);
    let (conn_id, first) = state.hub.add(&user.id, tx.clone());

    let hello = ServerEvent::Hello { user_id: user.id.clone() };
    if let Ok(text) = serde_json::to_string(&hello) {
        let _ = tx.try_send(text);
    }
    if first {
        broadcast_presence(&state, &user.id, true).await;
    }

    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            tokio::select! {
                next = rx.recv() => match next {
                    Some(text) => {
                        if sink.send(WsMessage::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                _ = ping.tick() => {
                    if sink.send(WsMessage::Ping(Bytes::new())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sink.close().await;
    });

    loop {
        let next = match tokio::time::timeout(IDLE_LIMIT, stream.next()).await {
            Ok(Some(Ok(message))) => message,
            _ => break,
        };
        match next {
            WsMessage::Text(text) => {
                if let Ok(event) = serde_json::from_str::<ClientEvent>(text.as_str()) {
                    handle(&state, &user, &tx, event).await;
                }
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    writer.abort();
    if let Some(user_id) = state.hub.remove(conn_id) {
        broadcast_presence(&state, &user_id, false).await;
    }
}

async fn handle(state: &AppState, user: &User, tx: &mpsc::Sender<String>, event: ClientEvent) {
    match event {
        ClientEvent::Ping => {
            if let Ok(text) = serde_json::to_string(&ServerEvent::Pong) {
                let _ = tx.try_send(text);
            }
        }
        ClientEvent::Typing {
            channel_id,
            conversation_id,
        } => {
            // At most a few typing pings per second per user.
            if !state.allow(&format!("typing:{}", user.id), 4, Duration::from_secs(2)) {
                return;
            }
            let audience = typing_audience(state, &user.id, channel_id.as_deref(), conversation_id.as_deref())
                .await
                .unwrap_or_default();
            let audience: Vec<String> = audience.into_iter().filter(|id| *id != user.id).collect();
            state.hub.send(
                &audience,
                &ServerEvent::Typing {
                    user: user.clone(),
                    channel_id,
                    conversation_id,
                },
            );
        }
    }
}

/// Who should see this user typing: the channel's community, or the other person.
async fn typing_audience(
    state: &AppState,
    user_id: &str,
    channel_id: Option<&str>,
    conversation_id: Option<&str>,
) -> ApiResult<Vec<String>> {
    let mut c = state.conn().await?;
    if let Some(id) = channel_id {
        let (community_id, _) = require_channel(&mut c, id, user_id).await?;
        community_member_ids(&mut c, &community_id).await
    } else if let Some(id) = conversation_id {
        Ok(vec![require_conversation(&mut c, id, user_id).await?])
    } else {
        Ok(Vec::new())
    }
}

async fn broadcast_presence(state: &AppState, user_id: &str, online: bool) {
    let lookup = async {
        let mut c = state.conn().await?;
        related_user_ids(&mut c, user_id).await
    };
    match lookup.await {
        Ok(audience) => state.hub.send(
            &audience,
            &ServerEvent::Presence {
                user_id: user_id.to_string(),
                online,
            },
        ),
        Err(error) => warn!("presence lookup failed: {error:?}"),
    }
}
