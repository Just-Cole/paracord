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
use std::time::Duration;

use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("paracord=info")),
        )
        .init();

    let cfg = Config::from_env();
    std::fs::create_dir_all(&cfg.data_dir)?;
    let db_path = cfg.data_dir.join("paracord.db");
    let conn = db::open(&db_path)?;
    info!("Database: {}", db_path.display());

    let state = AppState::new(cfg.clone(), conn);
    spawn_session_cleanup(state.clone());

    let app = routes::router(state.clone());
    let addr = SocketAddr::new(cfg.bind, cfg.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Pelican watches the console for this line to mark the server as started.
    info!("Paracord listening on http://{addr}");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("Paracord stopped");
    Ok(())
}

fn spawn_session_cleanup(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            tick.tick().await;
            let now = util::now_ms();
            let result = state
                .db(move |c| {
                    c.execute("DELETE FROM sessions WHERE expires_at < ?1", [now])?;
                    Ok(())
                })
                .await;
            if let Err(error) = result {
                warn!("session cleanup failed: {error:?}");
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
