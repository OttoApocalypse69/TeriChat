//! Health and readiness probes.
//!
//! Liveness never touches dependencies; readiness reports the database wiring
//! state.

use axum::{extract::State, Json};
use serde::Serialize;
use std::time::Duration;

use crate::errors::AppError;
use crate::state::AppState;

/// Liveness probe — never touches dependencies.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

/// Readiness probe — reports database wiring state.
#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    status: &'static str,
    database: &'static str,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "terichat-server",
    })
}

/// Readiness probe: reports database wiring state.
///
/// # Errors
///
/// Returns [`AppError::Unavailable`] when the database ping fails or exceeds
/// two seconds, including pool acquisition. Driver details stay private.
pub async fn ready(State(state): State<AppState>) -> Result<Json<ReadyResponse>, AppError> {
    match &state.pool {
        None => Ok(Json(ReadyResponse {
            status: "ok",
            database: "not_configured",
        })),
        Some(pool) => match tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query("SELECT 1").execute(pool),
        )
        .await
        {
            Ok(Ok(_)) => Ok(Json(ReadyResponse {
                status: "ok",
                database: "connected",
            })),
            Ok(Err(_)) | Err(_) => Err(AppError::Unavailable("database unavailable".to_owned())),
        },
    }
}
