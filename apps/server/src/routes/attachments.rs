//! Attachment HTTP adapters: raw-bytes upload plus byte-identical download.
//!
//! Thin adapters over `crate::attachments`. Message envelopes are UNCHANGED:
//! refs travel inside client JSON (`{text, attachments:[...]}`), never as new
//! envelope fields. Both routes are membership-checked with no oracle:
//! non-members see the same 404 as missing ids.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use http_body_util::{BodyExt, Limited};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::attachments::{self, MAX_ATTACHMENT_BYTES};
use crate::errors::AppError;
use crate::state::{AppState, Bearer};
use crate::workspaces;

/// `POST /v1/conversations/:id/attachments` query: filename/mime travel here
/// or via headers (`X-Filename`, `X-Mime-Type` / `Content-Type`).
#[derive(Debug, Deserialize, Default)]
struct UploadQuery {
    filename: Option<String>,
    mime: Option<String>,
    content_type: Option<String>,
}

/// Attachment view returned by the upload route.
#[derive(Debug, Serialize)]
struct AttachmentBody {
    id: Uuid,
    conversation_id: Uuid,
    filename: String,
    mime: String,
    size_bytes: i64,
    sha256: String,
    created_at: DateTime<Utc>,
}

impl From<&attachments::Attachment> for AttachmentBody {
    fn from(row: &attachments::Attachment) -> Self {
        Self {
            id: row.id,
            conversation_id: row.conversation_id,
            filename: row.filename.clone(),
            mime: row.mime.clone(),
            size_bytes: row.size_bytes,
            sha256: STANDARD.encode(&row.sha256),
            created_at: row.created_at,
        }
    }
}

fn first_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn upload_attachment(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(conversation_id): Path<Uuid>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<AttachmentBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Membership first (no oracle): channel gates and the conversation wall
    // both collapse to 404 so strangers cannot probe for channel existence.
    if let Some(channel) = workspaces::channel_by_conversation(pool, conversation_id)
        .await
        .map_err(|err| {
            tracing::error!("attachment channel lookup failed: {err}");
            AppError::Internal
        })?
    {
        let gate = async {
            let (_workspace, _role) =
                workspaces::get_workspace(pool, bearer.user_id(), channel.workspace_id).await?;
            workspaces::ensure_channel_participation(pool, channel.workspace_id, bearer.user_id())
                .await?;
            if !workspaces::can_send(pool, channel.id, bearer.user_id()).await? {
                return Err(workspaces::WorkspacesError::Forbidden);
            }
            Ok::<(), workspaces::WorkspacesError>(())
        }
        .await;
        if let Err(err) = gate {
            // No-oracle collapse is deliberate (strangers see 404 either
            // way), but backend failures must stay visible in logs.
            tracing::error!("attachment upload gate failed: {err}");
            return Err(AppError::NotFound("attachment not found".to_owned()));
        }
    }
    // Streaming cap: `Limited` rejects during transfer once the body
    // exceeds 10 MiB + 1 byte, so a multi-GB payload never buffers into
    // RAM. Client pre-checks are bypassable; this is the real gate.
    let capped = Limited::new(body, MAX_ATTACHMENT_BYTES + 1);
    let collected = capped
        .collect()
        .await
        .map_err(|_| AppError::PayloadTooLarge("attachment exceeds 10 MiB".to_owned()))?;
    let bytes = collected.to_bytes();
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(AppError::PayloadTooLarge(
            "attachment exceeds 10 MiB".to_owned(),
        ));
    }
    let filename = query
        .filename
        .filter(|value| !value.trim().is_empty())
        .or_else(|| first_header(&headers, "x-filename"))
        .ok_or_else(|| AppError::BadRequest("filename is required".to_owned()))?;
    let mime = query
        .mime
        .filter(|value| !value.trim().is_empty())
        .or(query.content_type.filter(|value| !value.trim().is_empty()))
        .or_else(|| first_header(&headers, "x-mime-type"))
        .or_else(|| {
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.split(';').next().unwrap_or(value).trim().to_owned())
        })
        .ok_or_else(|| AppError::BadRequest("mime is required".to_owned()))?;
    let stored = attachments::upload_attachment(
        pool,
        state.storage.as_ref(),
        bearer.user_id(),
        conversation_id,
        &filename,
        &mime,
        &bytes,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(AttachmentBody::from(&stored))))
}

async fn download_attachment(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(attachment_id): Path<Uuid>,
) -> Result<Response, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Resolve the row first so channel gates can run against the parent
    // conversation; every failure below is the same 404 (no oracle).
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT conversation_id FROM attachments WHERE id = $1")
            .bind(attachment_id)
            .fetch_optional(pool)
            .await
            .map_err(|err| {
                tracing::error!("attachment lookup failed: {err}");
                AppError::Internal
            })?;
    let Some((conversation_id,)) = row else {
        return Err(AppError::NotFound("attachment not found".to_owned()));
    };
    if let Some(channel) = workspaces::channel_by_conversation(pool, conversation_id)
        .await
        .map_err(|err| {
            tracing::error!("attachment channel lookup failed: {err}");
            AppError::Internal
        })?
    {
        let gate = async {
            let (_workspace, _role) =
                workspaces::get_workspace(pool, bearer.user_id(), channel.workspace_id).await?;
            workspaces::ensure_channel_participation(pool, channel.workspace_id, bearer.user_id())
                .await?;
            Ok::<(), workspaces::WorkspacesError>(())
        }
        .await;
        if let Err(err) = gate {
            // Same deliberate collapse as upload: 404 outward, error inward.
            tracing::error!("attachment download gate failed: {err}");
            return Err(AppError::NotFound("attachment not found".to_owned()));
        }
    }
    let (attachment, bytes) = attachments::download_attachment(
        pool,
        state.storage.as_ref(),
        bearer.user_id(),
        attachment_id,
    )
    .await?;
    // Integrity: stored bytes must match the row hash. FS/DB skew (bit-rot,
    // tamper, partial write) is a 500 with a log, never served as canonical.
    {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(&bytes);
        if digest.as_slice() != attachment.sha256.as_slice() {
            tracing::error!("attachment {attachment_id} bytes do not match stored sha256");
            return Err(AppError::Internal);
        }
        if i64::try_from(bytes.len()).unwrap_or(i64::MAX) != attachment.size_bytes {
            tracing::error!("attachment {attachment_id} bytes do not match stored size");
            return Err(AppError::Internal);
        }
    }
    let is_svg = attachment.mime.eq_ignore_ascii_case("image/svg+xml");
    let disposition = format!(
        "{}; filename=\"{}\"",
        if is_svg { "attachment" } else { "inline" },
        attachment.filename.replace('"', "_")
    );
    let response = (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                attachment.mime.parse().unwrap_or(mime::STAR_STAR.clone()),
            ),
            (
                header::CONTENT_DISPOSITION,
                disposition.parse().unwrap_or("inline".parse().unwrap()),
            ),
            // Stored upload bytes are user-supplied; never let a browser
            // sniff them into an executable context.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap()),
        ],
        bytes,
    )
        .into_response();
    Ok(response)
}

mod mime {
    use axum::http::HeaderValue;
    use std::sync::LazyLock;

    /// Fallback `*/*` when the stored MIME fails to parse (cannot happen for
    /// allowlisted rows, but handlers must never panic on stored data).
    pub static STAR_STAR: LazyLock<HeaderValue> = LazyLock::new(|| HeaderValue::from_static("*/*"));
}

/// Attachment routes: upload under a conversation plus id-addressed download.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/conversations/{id}/attachments",
            axum::routing::post(upload_attachment),
        )
        .route("/v1/attachments/{id}", get(download_attachment))
}
