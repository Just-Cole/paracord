use std::collections::{HashMap, VecDeque};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sqlx::MySqlPool;
use sqlx::pool::PoolConnection;

use crate::config::Config;
use crate::error::ApiResult;
use crate::realtime::Hub;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub hub: Hub,
    pub pool: MySqlPool,
    limiter: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(cfg: Config, pool: MySqlPool) -> Self {
        Self(Arc::new(Inner {
            cfg,
            hub: Hub::default(),
            pool,
            limiter: Mutex::new(HashMap::new()),
        }))
    }

    /// A database connection from the pool.
    pub async fn conn(&self) -> ApiResult<PoolConnection<sqlx::MySql>> {
        Ok(self.pool.acquire().await?)
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
