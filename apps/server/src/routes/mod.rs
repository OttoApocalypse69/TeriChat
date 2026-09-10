//! HTTP routes: thin merge point over per-domain adapters.
//!
//! Each domain module below owns its handlers plus request/response shapes
//! and exposes a `router()`; this module only merges them with probes, the
//! gateway, and the other domain routers. No handler logic lives here.

use axum::{routing::get, Router};

use crate::gateway;
use crate::health::{health, ready};
use crate::state::AppState;

pub mod auth;
pub mod messaging;
pub mod stats;
pub mod workspaces;

/// Build the full router by merging per-domain adapters.
///
/// Paths, status codes, and JSON shapes are unchanged from the former
/// monolithic `routes.rs`; this function only composes sub-routers.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .merge(auth::router())
        .merge(messaging::router())
        .merge(workspaces::router())
        .merge(stats::router())
        .route("/v1/gateway", get(gateway::gateway_handler))
        .merge(crate::session_management::router())
        .merge(crate::workspace_stats::router())
        .merge(crate::moderation::router())
        .merge(crate::ledger::router())
        .with_state(state)
}
