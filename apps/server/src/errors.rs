//! Shared HTTP error and response handling.
//!
//! [`AppError`] is the single error type every handler returns; its
//! [`IntoResponse`] impl fixes the status/code/message envelope, and the
//! `From` impls map domain errors without leaking internals.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

use crate::attachments;
use crate::auth;
use crate::ledger;
use crate::messaging;
use crate::stats;
use crate::workspaces;

/// Shared JSON error shape for the API.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

/// Machine-readable error detail.
#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

/// Typed application error.
#[derive(Debug)]
pub enum AppError {
    /// Caller-supplied value rejected (taken handle/email, bad input).
    BadRequest(String),
    /// Missing or rejected bearer credentials.
    Unauthorized,
    /// Caller is not a conversation member (also covers missing rows).
    Forbidden,
    /// Caller is not a workspace member or lacks workspace permission.
    /// Carries a safe message (never workspace existence details).
    Denied(String),
    /// Named object (e.g. invite) is unknown or unusable.
    NotFound(String),
    /// Body exceeds the accepted limit (attachments: 10 MiB).
    PayloadTooLarge(String),
    /// MIME type outside the accepted allowlist.
    UnsupportedMediaType(String),
    /// A database-backed route called without a configured database.
    NoDatabase,
    /// Readiness dependency unavailable.
    Unavailable(String),
    /// Internal failure. Details go to logs, never to callers.
    Internal,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "invalid or missing credentials".to_owned(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "not a conversation member".to_owned(),
            ),
            Self::Denied(message) => (StatusCode::FORBIDDEN, "forbidden", message),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message),
            Self::PayloadTooLarge(message) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
            }
            Self::UnsupportedMediaType(message) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                message,
            ),
            Self::NoDatabase => (
                StatusCode::SERVICE_UNAVAILABLE,
                "no_database",
                "account API requires DATABASE_URL".to_owned(),
            ),
            Self::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", message),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error".to_owned(),
            ),
        };
        let body = Json(ErrorBody {
            error: ErrorDetail { code, message },
        });
        (status, body).into_response()
    }
}

impl From<auth::AuthError> for AppError {
    fn from(err: auth::AuthError) -> Self {
        match err {
            auth::AuthError::HandleTaken => Self::BadRequest("handle is taken".to_owned()),
            auth::AuthError::EmailTaken => Self::BadRequest("email is registered".to_owned()),
            auth::AuthError::InvalidInput(detail) => Self::BadRequest(detail),
            auth::AuthError::InvalidCredentials | auth::AuthError::InvalidToken => {
                Self::Unauthorized
            }
            other => {
                // `Display` for these variants carries no credential material.
                tracing::error!("auth backend failure: {other}");
                Self::Internal
            }
        }
    }
}

impl From<workspaces::WorkspacesError> for AppError {
    fn from(err: workspaces::WorkspacesError) -> Self {
        match err {
            // No existence oracle: outsiders get a workspace-shaped wall.
            workspaces::WorkspacesError::NotMember => {
                Self::Denied("not a workspace member".to_owned())
            }
            workspaces::WorkspacesError::Forbidden => {
                Self::Denied("insufficient workspace permission".to_owned())
            }
            workspaces::WorkspacesError::Banned => {
                Self::Denied("banned from this workspace".to_owned())
            }
            workspaces::WorkspacesError::InviteRejected => {
                Self::NotFound("invite is invalid, expired, or fully used".to_owned())
            }
            workspaces::WorkspacesError::BadInput(detail) => Self::BadRequest(detail),
            workspaces::WorkspacesError::Database(_) => {
                tracing::error!("workspace backend failure: {err}");
                Self::Internal
            }
        }
    }
}
impl From<messaging::MessagingError> for AppError {
    fn from(err: messaging::MessagingError) -> Self {
        match err {
            messaging::MessagingError::NotMember => Self::Forbidden,
            messaging::MessagingError::EmptyCiphertext
            | messaging::MessagingError::CiphertextTooLarge => Self::BadRequest(err.to_string()),
            messaging::MessagingError::Database(_) => {
                tracing::error!("messaging backend failure: {err}");
                Self::Internal
            }
        }
    }
}

impl From<attachments::AttachmentError> for AppError {
    fn from(err: attachments::AttachmentError) -> Self {
        match err {
            // No existence oracle: non-members see the same 404 as missing ids.
            attachments::AttachmentError::NotFound => {
                Self::NotFound("attachment not found".to_owned())
            }
            attachments::AttachmentError::InvalidInput(detail) => Self::BadRequest(detail),
            attachments::AttachmentError::TooLarge => {
                Self::PayloadTooLarge("attachment exceeds 10 MiB".to_owned())
            }
            attachments::AttachmentError::UnsupportedMime => {
                Self::UnsupportedMediaType("unsupported media type".to_owned())
            }
            // Outward signal stays a generic 500, but the inner cause must
            // reach operator logs: "storage/database error" alone cannot
            // distinguish a full disk from a missing file or a dead pool.
            attachments::AttachmentError::Database(db) => {
                tracing::error!("attachment database failure: {db}");
                Self::Internal
            }
            attachments::AttachmentError::Storage(detail) => {
                tracing::error!("attachment storage failure: {detail}");
                Self::Internal
            }
        }
    }
}

impl From<stats::StatsError> for AppError {
    fn from(err: stats::StatsError) -> Self {
        match err {
            stats::StatsError::NotMember => Self::Forbidden,
            stats::StatsError::BadInput(detail) => Self::BadRequest(detail),
            stats::StatsError::Database(inner) => {
                tracing::error!("stats backend failure: {inner}");
                Self::Internal
            }
        }
    }
}

impl From<ledger::LedgerError> for AppError {
    fn from(err: ledger::LedgerError) -> Self {
        match err {
            ledger::LedgerError::BadInput(detail) => Self::BadRequest(detail),
            // No existence oracle: cross-wallet probes get a uniform wall.
            ledger::LedgerError::Forbidden => Self::Denied("not your wallet".to_owned()),
            ledger::LedgerError::Invariant(detail) => {
                tracing::error!("ledger invariant violated: {detail}");
                Self::Internal
            }
            ledger::LedgerError::Database(inner) => {
                tracing::error!("ledger backend failure: {inner}");
                Self::Internal
            }
        }
    }
}
