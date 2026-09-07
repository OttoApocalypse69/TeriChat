//! Shared application state and request identity.
//!
//! [`AppState`] is the pool/hub pair every handler receives (`pool: None`
//! means the probes-only boot). [`Bearer`] extracts the caller's session from
//! the `Authorization` header for handlers that need identity.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::auth;
use crate::errors::AppError;
use crate::messaging;

/// Shared server state: the optional database pool plus the realtime fan-out
/// hub. `pool: None` means the probes-only boot (no `DATABASE_URL`).
#[derive(Clone)]
pub struct AppState {
    pub pool: Option<sqlx::PgPool>,
    pub hub: broadcast::Sender<messaging::OutboxEntry>,
}

/// Hub capacity: live burst buffer. Overflow drops to resume (`Lagged`
/// receivers re-anchor from the database), never to data loss.
pub const HUB_CAPACITY: usize = 1024;

/// Authenticated request identity plus the raw bearer token, available only
/// to handlers that need to hash it (logout). Handlers that only need the
/// identity use [`Bearer::user_id`].
pub struct Bearer {
    session: auth::AuthSession,
    token: String,
}

impl Bearer {
    /// Requesting account id.
    #[must_use]
    pub fn user_id(&self) -> Uuid {
        self.session.user_id
    }

    /// Raw bearer token. Handle like a password: hash, never log.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Session id for responses that echo it (e.g. logout).
    #[must_use]
    pub fn session_id(&self) -> Uuid {
        self.session.session_id
    }
}

impl FromRequestParts<AppState> for Bearer {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .filter(|token| !token.is_empty())
            .ok_or(AppError::Unauthorized)?;
        // The failure log carries no token material; the handle is not secret.
        let session = auth::authenticate(pool, token).await.map_err(|err| {
            tracing::debug!("bearer auth rejected: {err}");
            AppError::from(err)
        })?;
        tracing::debug!(
            user = %session.handle,
            session = %session.session_id,
            "bearer authenticated"
        );
        Ok(Self {
            session,
            token: token.to_owned(),
        })
    }
}
