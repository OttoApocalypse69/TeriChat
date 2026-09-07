//! Health and readiness probes.
//!
//! Liveness never touches dependencies; readiness reports the database wiring
//! state.

use axum::{extract::State, Json};
use serde::Serialize;

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

pub async fn ready(State(state): State<AppState>) -> Result<Json<ReadyResponse>, AppError> {
    match &state.pool {
        None => Ok(Json(ReadyResponse {
            status: "ok",
            database: "not_configured",
        })),
        Some(pool) => {
            sqlx::query("SELECT 1")
                .execute(pool)
                .await
                .map_err(|err| AppError::Unavailable(format!("database ping failed: {err}")))?;
            Ok(Json(ReadyResponse {
                status: "ok",
                database: "connected",
            }))
        }
    }
}
