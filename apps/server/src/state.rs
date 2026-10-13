//! Shared application state and request identity.
//!
//! [`AppState`] is the pool/hub/storage triple every handler receives
//! (`pool: None` means the probes-only boot). [`Bearer`] extracts the
//! caller's session from the `Authorization` header for handlers that need
//! identity.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use std::sync::Arc;
use tokio::sync::{broadcast, Semaphore};
use uuid::Uuid;

use crate::attachments::FilesystemAttachmentStorage;
use crate::auth;
use crate::errors::AppError;
use crate::messaging;

/// Shared server state: the optional database pool, the realtime fan-out
/// hub, and the attachment byte store. `pool: None` means the probes-only
/// boot (no `DATABASE_URL`).
#[derive(Clone)]
pub struct AppState {
    pub pool: Option<sqlx::PgPool>,
    pub hub: broadcast::Sender<messaging::OutboxEntry>,
    /// Attachment bytes. Filesystem-backed by default (`ATTACHMENTS_DIR`);
    /// the trait keeps a later S3 move handler-local.
    pub storage: Arc<FilesystemAttachmentStorage>,
    /// Upload slots ([`MAX_CONCURRENT_UPLOADS`]): bounds how many 10 MiB
    /// bodies are buffered at once across all requests.
    pub upload_slots: Arc<Semaphore>,
    /// Download slots ([`MAX_CONCURRENT_DOWNLOADS`]): each permit lives
    /// with its response body, so slow readers cannot pile up blobs.
    pub download_slots: Arc<Semaphore>,
}

/// Uploads buffered concurrently. Each holds at most 10 MiB plus the
/// storage copy, so the aggregate stays near 160 MiB under any load.
pub const MAX_CONCURRENT_UPLOADS: usize = 8;

/// Downloads in flight. Each holds at most one 10 MiB blob until its
/// response is sent or dropped, so the aggregate stays near 160 MiB.
pub const MAX_CONCURRENT_DOWNLOADS: usize = 16;

impl AppState {
    /// Production constructor: filesystem storage rooted at `attachments_dir`.
    #[must_use]
    pub fn new(
        pool: Option<sqlx::PgPool>,
        hub: broadcast::Sender<messaging::OutboxEntry>,
        attachments_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            pool,
            hub,
            storage: Arc::new(FilesystemAttachmentStorage::new(attachments_dir)),
            upload_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_UPLOADS)),
            download_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS)),
        }
    }

    /// Test constructor: filesystem storage rooted at `dir`.
    #[must_use]
    pub fn test(
        pool: sqlx::PgPool,
        hub: broadcast::Sender<messaging::OutboxEntry>,
        dir: std::path::PathBuf,
    ) -> Self {
        Self {
            pool: Some(pool),
            hub,
            storage: Arc::new(FilesystemAttachmentStorage::new(dir)),
            upload_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_UPLOADS)),
            download_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS)),
        }
    }
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
