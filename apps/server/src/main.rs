//! `TeriChat` modular-monolith API server (Alpha 0).
//!
//! Wiring only: `main` builds the configuration, connects the pool via
//! bootstrap, shares application state, mounts the router, spawns the
//! background workers, and serves with a bounded graceful shutdown. Probes live in
//! the health module, shared errors in the errors module.
//!
//! Serves liveness/readiness probes and the account API under `/v1/auth/*`.
//! Configuration comes from the config module; without `DATABASE_URL` the
//! server boots bare with only the probes live.

#![forbid(unsafe_code)]

use std::net::SocketAddr;

use terichat_server::{bootstrap, build_router, config, AppState, HUB_CAPACITY};
use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    let config = config::Config::from_env().unwrap_or_else(|err| {
        eprintln!("configuration error: {err}");
        std::process::exit(1);
    });

    bootstrap::init_tracing(&config);

    let pool = bootstrap::connect_pool(&config).await;

    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    let workers = bootstrap::spawn_workers(pool.as_ref(), &hub);

    let addr = SocketAddr::from((config.bind_addr, config.port));
    let listener = bootstrap::bind_listener(addr).await;
    tracing::info!(%addr, "terichat-server listening");
    let app = build_router(AppState {
        pool: pool.clone(),
        hub,
    });
    // Drain order: HTTP, then workers, then the pool. See bootstrap docs.
    let served = bootstrap::serve_until(
        listener,
        app,
        bootstrap::shutdown_signal(),
        bootstrap::HTTP_DRAIN,
    )
    .await;
    workers.shutdown(bootstrap::WORKER_DRAIN).await;
    if let Some(pool) = &pool {
        bootstrap::close_pool(pool, bootstrap::POOL_CLOSE).await;
    }
    match served {
        Ok(_) => tracing::info!("terichat-server stopped"),
        Err(err) => {
            tracing::error!("server accept loop failed: {err}");
            std::process::exit(1);
        }
    }
}
