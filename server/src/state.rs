use std::collections::{HashMap, VecDeque};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::realtime::Hub;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub hub: Hub,
    db: Arc<Mutex<Connection>>,
    limiter: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(cfg: Config, conn: Connection) -> Self {
        Self(Arc::new(Inner {
            cfg,
            hub: Hub::default(),
            db: Arc::new(Mutex::new(conn)),
            limiter: Mutex::new(HashMap::new()),
        }))
    }

    /// Runs database work on a blocking thread. SQLite has one writer anyway,
    /// so a single shared connection keeps things simple.
    pub async fn db<T, F>(&self, work: F) -> ApiResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> ApiResult<T> + Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = db.lock().map_err(|_| ApiError::internal("database lock poisoned"))?;
            work(&mut conn)
        })
        .await
        .map_err(ApiError::internal)?
    }

    /// Sliding-window rate limit: at most `max` hits per `window` for `key`.
    pub fn allow(&self, key: &str, max: usize, window: Duration) -> bool {
        let now = Instant::now();
        let mut map = self.limiter.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, hits| hits.back().is_some_and(|t| now.duration_since(*t) < window));
        }
        let hits = map.entry(key.to_string()).or_default();
        while hits.front().is_some_and(|t| now.duration_since(*t) >= window) {
            hits.pop_front();
        }
        if hits.len() >= max {
            return false;
        }
        hits.push_back(now);
        true
    }
}
