//! Server bootstrap: the startup pieces behind [`main`](crate::main)'s wiring.
//!
//! Database pool plus forward-only migrations, background worker spawn,
//! listener bind, and graceful shutdown. No domain logic lives here.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::broadcast;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::Config;
use crate::messaging;
use crate::stats;

/// Embedded, forward-only `SQLx` migrations from the workspace `migrations/`
/// directory. The macro validates at compile time that the directory exists
/// and parses, so a missing/broken migration fails the build — no database
/// connection required.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Shared database pool for request handlers and boot migration.
///
/// # Errors
///
/// Returns [`sqlx::Error`] when the connection cannot be established.
pub async fn db_pool(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
}

/// Install the process-wide tracing subscriber from configuration.
pub fn init_tracing(config: &Config) {
    fmt()
        .with_env_filter(
            EnvFilter::try_new(&config.rust_log).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();
}

/// Connect the configured database and apply pending migrations.
///
/// Returns [`None`] when no `DATABASE_URL` is set: the server still boots
/// bare with only the probes live.
///
/// # Panics
///
/// Panics when a configured database is unreachable or migrations fail;
/// the server cannot serve then.
pub async fn connect_pool(config: &Config) -> Option<sqlx::PgPool> {
    // Forward-only schema migration, applied once per boot when a database is
    // configured. Without `DATABASE_URL` the server still boots bare.
    match &config.database_url {
        None => None,
        Some(url) => {
            let pool = db_pool(url).await.expect("connect database");
            MIGRATOR
                .run(&pool)
                .await
                .expect("apply pending SQLx migrations");
            tracing::info!("database migrations applied");
            Some(pool)
        }
    }
}

/// Spawn the background workers when a database is configured.
///
/// At-least-once fan-out; the next boot re-claims anything unmarked.
pub fn spawn_workers(pool: Option<&sqlx::PgPool>, hub: &broadcast::Sender<messaging::OutboxEntry>) {
    if let Some(pool) = pool {
        // At-least-once fan-out; the next boot re-claims anything unmarked.
        tokio::spawn(messaging::outbox_worker(pool.clone(), hub.clone()));
        // Stats fold: at-least-once too, deduped on the outbox event id.
        tokio::spawn(stats::stats_worker(pool.clone()));
    }
}

/// Bind the TCP listener for the configured address.
///
/// # Panics
///
/// Panics when the address cannot be bound; the server cannot serve then.
pub async fn bind_listener(addr: SocketAddr) -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind server address")
}

/// Wait for Ctrl-C, then let the server drain in flight work.
pub async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
