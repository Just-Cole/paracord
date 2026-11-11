use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use paracord_shared::ServerEvent;
use tokio::sync::mpsc;

/// Messages queued for one socket. If a client falls this far behind it is
/// skipped rather than letting memory grow; it refetches when it reconnects.
pub const SOCKET_QUEUE: usize = 256;

struct Conn {
    user_id: String,
    tx: mpsc::Sender<String>,
}

/// Tracks every open WebSocket and who it belongs to.
#[derive(Default)]
pub struct Hub {
    next_id: AtomicU64,
    conns: RwLock<HashMap<u64, Conn>>,
}

impl Hub {
    /// Registers a socket. Returns its id and whether it's the user's first.
    pub fn add(&self, user_id: &str, tx: mpsc::Sender<String>) -> (u64, bool) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut conns = self.conns.write().unwrap_or_else(|e| e.into_inner());
        let first = !conns.values().any(|c| c.user_id == user_id);
        conns.insert(
            id,
            Conn {
                user_id: user_id.to_string(),
                tx,
            },
        );
        (id, first)
    }

    /// Removes a socket. Returns the user id if that was their last socket.
    pub fn remove(&self, id: u64) -> Option<String> {
        let mut conns = self.conns.write().unwrap_or_else(|e| e.into_inner());
        let conn = conns.remove(&id)?;
        let still_here = conns.values().any(|c| c.user_id == conn.user_id);
        (!still_here).then_some(conn.user_id)
    }

    pub fn online_users(&self) -> HashSet<String> {
        let conns = self.conns.read().unwrap_or_else(|e| e.into_inner());
        conns.values().map(|c| c.user_id.clone()).collect()
    }

    pub fn send<'a>(&self, users: impl IntoIterator<Item = &'a String>, event: &ServerEvent) {
        let allow: HashSet<&String> = users.into_iter().collect();
        if allow.is_empty() {
            return;
        }
        let Ok(payload) = serde_json::to_string(event) else {
            return;
        };
        let conns = self.conns.read().unwrap_or_else(|e| e.into_inner());
        for conn in conns.values() {
            if allow.contains(&conn.user_id) {
                let _ = conn.tx.try_send(payload.clone());
            }
        }
    }

    pub fn send_one(&self, user_id: &String, event: &ServerEvent) {
        self.send([user_id], event);
    }
}
