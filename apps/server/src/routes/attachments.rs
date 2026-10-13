//! Attachment HTTP adapters: raw-bytes upload plus byte-identical download.
//!
//! Thin adapters over `crate::attachments`. Message envelopes are UNCHANGED:
//! refs travel inside client JSON (`{text, attachments:[...]}`), never as new
//! envelope fields. Both routes are membership-checked with no oracle:
//! non-members see the same 404 as missing ids.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
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

/// Decode a percent-encoded UTF-8 value (`encodeURIComponent` output).
/// `None` for a truncated or non-hex escape or for bytes that are not UTF-8.
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = raw.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Filename from the `X-Filename` header. Header values are bytes, and
/// browsers refuse to set anything above U+00FF, so clients send Unicode
/// names percent-encoded with `X-Filename-Encoding: percent`. Without that
/// marker the value is taken literally (older clients, plain ASCII names).
fn header_filename(headers: &HeaderMap) -> Result<Option<String>, AppError> {
    let Some(raw) = first_header(headers, "x-filename") else {
        return Ok(None);
    };
    match first_header(headers, "x-filename-encoding").as_deref() {
        None => Ok(Some(raw)),
        Some(encoding) if encoding.eq_ignore_ascii_case("percent") => percent_decode(&raw)
            .map(Some)
            .ok_or_else(|| AppError::BadRequest("filename encoding is invalid".to_owned())),
        Some(_) => Err(AppError::BadRequest(
            "unsupported filename encoding".to_owned(),
        )),
    }
}

/// `Content-Disposition` for a stored filename (RFC 6266): an ASCII
/// `filename` fallback plus the exact UTF-8 name in `filename*`, so names
/// outside visible ASCII never make the header unbuildable.
fn content_disposition(disposition: &str, filename: &str) -> String {
    use std::fmt::Write as _;
    let fallback: String = filename
        .chars()
        .map(|c| {
            if c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::with_capacity(filename.len() * 3);
    for byte in filename.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    format!("{disposition}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

fn first_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Channel gates for an upload into `conversation_id`: workspace
/// membership, channel participation, and `send_messages`. A no-op for
/// DMs (the participant check runs in the storage transaction). Every
/// failure collapses to 404 so strangers cannot probe for channels.
async fn channel_send_gate(
    pool: &sqlx::PgPool,
    conversation_id: Uuid,
    user_id: Uuid,
) -> Result<(), AppError> {
    let Some(channel) = workspaces::channel_by_conversation(pool, conversation_id)
        .await
        .map_err(|err| {
            tracing::error!("attachment channel lookup failed: {err}");
            AppError::Internal
        })?
    else {
        return Ok(());
    };
    let gate = async {
        let (_workspace, _role) =
            workspaces::get_workspace(pool, user_id, channel.workspace_id).await?;
        workspaces::ensure_channel_participation(pool, channel.workspace_id, user_id).await?;
        if !workspaces::can_send(pool, channel.id, user_id).await? {
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
    Ok(())
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
    // Membership first (no oracle), before any body bytes are read.
    channel_send_gate(pool, conversation_id, bearer.user_id()).await?;
    // Aggregate bound: each in-flight upload may buffer up to 10 MiB, so a
    // fixed number of slots caps total upload memory however many parallel
    // requests a member opens. Waiters hold no body bytes.
    let _slot = state
        .upload_slots
        .acquire()
        .await
        .map_err(|_| AppError::Internal)?;
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
    // Re-check after the (possibly slow) transfer: a role demotion or
    // `send_messages` denial committed mid-upload must not still store the
    // file. This leaves the same check-then-insert window as message send.
    channel_send_gate(pool, conversation_id, bearer.user_id()).await?;
    let filename = query
        .filename
        .filter(|value| !value.trim().is_empty())
        .or(header_filename(&headers)?)
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
    let disposition = content_disposition(
        if is_svg { "attachment" } else { "inline" },
        &attachment.filename,
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
                // Always visible ASCII (see `content_disposition`); the
                // fallback still never downgrades an SVG to `inline`.
                disposition
                    .parse()
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_filenames_decode_and_malformed_ones_are_refused() {
        assert_eq!(
            percent_decode("%E5%A0%B1%E5%91%8A.pdf").as_deref(),
            Some("報告.pdf")
        );
        assert_eq!(percent_decode("plain.png").as_deref(), Some("plain.png"));
        assert_eq!(percent_decode("100%25.png").as_deref(), Some("100%.png"));
        assert_eq!(percent_decode("bad%2"), None);
        assert_eq!(percent_decode("bad%zz.png"), None);
        assert_eq!(percent_decode("%FF.png"), None);
    }

    #[test]
    fn header_filename_honours_the_encoding_marker() {
        let mut headers = HeaderMap::new();
        headers.insert("x-filename", HeaderValue::from_static("100%25.png"));
        assert_eq!(
            header_filename(&headers).expect("literal").as_deref(),
            Some("100%25.png")
        );
        headers.insert("x-filename-encoding", HeaderValue::from_static("percent"));
        assert_eq!(
            header_filename(&headers).expect("decoded").as_deref(),
            Some("100%.png")
        );
        headers.insert("x-filename-encoding", HeaderValue::from_static("base64"));
        assert!(header_filename(&headers).is_err());
    }

    #[test]
    fn disposition_is_ascii_with_an_exact_utf8_name() {
        let value = content_disposition("attachment", "報告 \"q\".svg");
        assert!(HeaderValue::from_str(&value).is_ok(), "{value}");
        assert_eq!(
            value,
            "attachment; filename=\"__ _q_.svg\"; filename*=UTF-8''%E5%A0%B1%E5%91%8A%20%22q%22.svg"
        );
    }
}
