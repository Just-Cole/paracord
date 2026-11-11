/// Builds SQL text at runtime. Only for queries assembled from fixed pieces
/// (column lists, table names); user input always goes through `.bind()`.
macro_rules! sql {
    ($($t:tt)*) => { sqlx::AssertSqlSafe(format!($($t)*)) };
}

mod access;
mod auth;
mod config;
mod db;
mod error;
mod realtime;
mod routes;
mod state;
mod util;
mod web;
mod ws;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("paracord=info")),
        )
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            error!("{message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let cfg = Config::from_env()?;
    let pool = db::connect(&cfg.db)
        .await
        .map_err(|e| format!("Database setup failed: {e}"))?;

    let state = AppState::new(cfg.clone(), pool);
    spawn_session_cleanup(state.clone());

    let app = routes::router(state.clone());
    let addr = SocketAddr::new(cfg.bind, cfg.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Can't listen on {addr}: {e}"))?;
    // Pelican watches the console for this line to mark the server as started.
    info!("Paracord listening on http://{addr}");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("Server error: {e}"))?;
    state.pool.close().await;
    info!("Paracord stopped");
    Ok(())
}

fn spawn_session_cleanup(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            tick.tick().await;
            let result = sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
                .bind(util::now_ms())
                .execute(&state.pool)
                .await;
            if let Err(error) = result {
                warn!("session cleanup failed: {error}");
            }
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Shutting down");
}
