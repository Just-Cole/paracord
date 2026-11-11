//! Connecting to MySQL and keeping the schema up to date.

use std::time::Duration;

use sqlx::MySqlPool;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use tracing::{info, warn};

use crate::config::DbConfig;

/// SQL files in server/migrations, compiled into the binary. Each runs once,
/// in order, and is recorded in the `_sqlx_migrations` table.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn options(cfg: &DbConfig) -> Result<MySqlConnectOptions, sqlx::Error> {
    let options = match cfg {
        DbConfig::Url(url) => url.parse::<MySqlConnectOptions>()?,
        DbConfig::Parts {
            host,
            port,
            database,
            username,
            password,
        } => MySqlConnectOptions::new()
            .host(host)
            .port(*port)
            .database(database)
            .username(username)
            .password(password),
    };
    Ok(options.charset("utf8mb4"))
}

/// Connects (waiting for the database if it isn't up yet) and runs migrations.
pub async fn connect(cfg: &DbConfig) -> Result<MySqlPool, sqlx::Error> {
    let options = options(cfg)?;
    let mut delay = Duration::from_secs(2);
    let pool = loop {
        match MySqlPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
        {
            Ok(pool) => break pool,
            Err(error) => {
                warn!(
                    "Can't reach the database at {} yet ({error}). Retrying in {}s.",
                    cfg.describe(),
                    delay.as_secs()
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    };
    info!("Connected to the database at {}", cfg.describe());
    MIGRATOR.run(&pool).await.map_err(|e| sqlx::Error::Migrate(Box::new(e)))?;
    Ok(pool)
}
