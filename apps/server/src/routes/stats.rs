//! Stats HTTP adapters: caller-scoped rollups.
//!
//! Thin adapters over `crate::stats`. Counts only, never content, and only
//! the caller's own unless membership-checked. No behavior changes.

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::AppError;
use crate::state::{AppState, Bearer};
use crate::stats;

async fn get_own_stats(
    State(state): State<AppState>,
    bearer: Bearer,
) -> Result<Json<StatsBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Scoped to the caller by construction: only their own id is queried.
    let rollup = stats::own_stats(pool, bearer.user_id()).await?;
    Ok(Json(StatsBody {
        user_id: rollup.user_id,
        message_count: rollup.message_count,
        last_message_at: rollup.last_message_at,
    }))
}

async fn get_conversation_stats(
    State(state): State<AppState>,
    bearer: Bearer,
    Query(params): Query<ConversationStatsParams>,
) -> Result<Json<ConversationStatsBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Membership-checked inside; outsiders get `Forbidden`, never a count.
    let scoped = stats::conversation_stats(pool, bearer.user_id(), params.conversation_id).await?;
    Ok(Json(ConversationStatsBody {
        user_id: scoped.user_id,
        conversation_id: scoped.conversation_id,
        message_count: scoped.message_count,
        last_message_at: scoped.last_message_at,
    }))
}

/// `GET /v1/stats/me` view: the caller's own message rollup. Counts only,
/// never content — and only the caller's own.
#[derive(Debug, Serialize)]
struct StatsBody {
    user_id: Uuid,
    message_count: i64,
    last_message_at: Option<DateTime<Utc>>,
}

/// `GET /v1/stats/conversation` view: the caller's own sends in one conversation.
#[derive(Debug, Serialize)]
struct ConversationStatsBody {
    user_id: Uuid,
    conversation_id: Uuid,
    message_count: i64,
    last_message_at: Option<DateTime<Utc>>,
}

/// `GET /v1/stats/conversation` query.
#[derive(Debug, Deserialize)]
struct ConversationStatsParams {
    conversation_id: Uuid,
}

/// Stats routes under `/v1/stats/*`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/stats/me", get(get_own_stats))
        .route("/v1/stats/conversation", get(get_conversation_stats))
}
