//! Account-owned session management HTTP routes.
use crate::state::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new()
}
