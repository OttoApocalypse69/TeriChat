//! `TeriChat` modular-monolith API server library (Alpha 0).
//!
//! Shared wiring for the binary and integration tests: domain modules,
//! application state, router construction, and bootstrap. The binary keeps
//! only runtime wiring; HTTP adapters live under `routes`, domain logic in
//! domain modules.

#![forbid(unsafe_code)]

pub mod auth;
pub mod bootstrap;
pub mod config;
pub mod errors;
pub mod gateway;
pub mod health;
pub mod ledger;
pub mod messaging;
pub mod moderation;
pub mod password;
pub mod routes;
pub mod session_management;
pub mod state;
pub mod stats;
pub mod workspace_stats;
pub mod workspaces;

pub use bootstrap::{db_pool, MIGRATOR};
pub use routes::build_router;
pub use state::{AppState, HUB_CAPACITY};
