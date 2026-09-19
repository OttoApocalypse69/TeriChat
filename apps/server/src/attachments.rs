//! Async attachments v1: original-bytes storage with membership-checked access.
//!
//! The server stores ORIGINAL bytes on disk plus one `attachments` row
//! (conversation, uploader, filename, mime, size, sha256). Message envelopes
//! are UNCHANGED: refs travel inside client JSON (`{text, attachments:[...]}`),
//! never as new envelope fields. Per-attachment keys and client-side previews
//! are a later slice; this module holds bytes only.
//!
//! Storage is behind [`AttachmentStorage`] so a later S3 backend can replace
//! [`FilesystemAttachmentStorage`] without touching handlers.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;

/// Max attachment bytes accepted (10 MiB). Larger uploads get HTTP 413.
pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;

/// Stored attachment row.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// Attachment id (`UUIDv7`).
    pub id: Uuid,
    /// Owning conversation.
    pub conversation_id: Uuid,
    /// Uploading account.
    pub uploader_id: Uuid,
    /// Client-supplied filename (path separators stripped).
    pub filename: String,
    /// Client-supplied MIME type (allowlisted).
    pub mime: String,
    /// Byte length of the stored object.
    pub size_bytes: i64,
    /// SHA-256 of the stored bytes.
    pub sha256: Vec<u8>,
    /// Upload time.
    pub created_at: DateTime<Utc>,
}

/// Typed attachment error. `NotFound` covers both missing rows and
/// non-members (no existence oracle): callers see 404 either way.
#[derive(Debug)]
pub enum AttachmentError {
    /// Unknown id, or the caller may not access it (same signal).
    NotFound,
    /// Caller-supplied value rejected (empty bytes, bad filename).
    InvalidInput(String),
    /// Body exceeds [`MAX_ATTACHMENT_BYTES`].
    TooLarge,
    /// MIME type outside the v1 allowlist.
    UnsupportedMime,
    /// Database failure (logged by the HTTP layer, never shown verbatim).
    Database(sqlx::Error),
    /// Filesystem/object failure (logged, never shown verbatim).
    Storage(String),
}

impl std::fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "attachment not found"),
            Self::InvalidInput(detail) => write!(f, "{detail}"),
            Self::TooLarge => write!(f, "attachment too large"),
            Self::UnsupportedMime => write!(f, "unsupported media type"),
            Self::Database(_) => write!(f, "database error"),
            Self::Storage(_) => write!(f, "storage error"),
        }
    }
}

impl std::error::Error for AttachmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(err) => Some(err),
            _ => None,
        }
    }
}

impl From<crate::messaging::MessagingError> for AttachmentError {
    fn from(err: crate::messaging::MessagingError) -> Self {
        match err {
            crate::messaging::MessagingError::Database(db) => Self::Database(db),
            // `is_member` only fails with `Database`; any other membership
            // failure is a backend surprise, logged by the HTTP layer.
            other => Self::Storage(other.to_string()),
        }
    }
}

/// v1 MIME allowlist: images, PDF, ZIP, audio/*, video/*.
/// Anything else is rejected with HTTP 415.
#[must_use]
pub fn is_allowed_mime(mime: &str) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    if mime.starts_with("image/") || mime.starts_with("audio/") || mime.starts_with("video/") {
        return true;
    }
    matches!(mime.as_str(), "application/pdf" | "application/zip")
}

/// Strip path separators from a client filename; reject empties and overlong.
fn sanitize_filename(raw: &str) -> Result<String, AttachmentError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AttachmentError::InvalidInput(
            "filename must not be empty".to_owned(),
        ));
    }
    if trimmed.len() > 255 {
        return Err(AttachmentError::InvalidInput(
            "filename too long".to_owned(),
        ));
    }
    let cleaned: String = trimmed
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    // Reject `.` / `..` after cleaning to keep storage keys flat.
    if cleaned == "." || cleaned == ".." {
        return Err(AttachmentError::InvalidInput(
            "filename must not be empty".to_owned(),
        ));
    }
    Ok(cleaned)
}

/// Byte store behind a trait so a later S3 backend can replace the
/// filesystem without touching handlers or domain logic.
pub trait AttachmentStorage: Send + Sync + std::fmt::Debug {
    /// Persist `bytes` under `id`.
    fn put(
        &self,
        id: Uuid,
        bytes: &[u8],
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
    /// Load the bytes stored under `id`.
    fn get(&self, id: Uuid) -> impl std::future::Future<Output = Result<Vec<u8>, String>> + Send;
    /// Delete the object stored under `id` (best-effort orphan cleanup).
    fn delete(&self, id: Uuid) -> impl std::future::Future<Output = Result<(), String>> + Send;
}

/// Filesystem implementation: one file per attachment id.
#[derive(Debug, Clone)]
pub struct FilesystemAttachmentStorage {
    dir: PathBuf,
}

impl FilesystemAttachmentStorage {
    /// Storage rooted at `dir` (created on first write).
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path_for(&self, id: Uuid) -> PathBuf {
        self.dir.join(id.simple().to_string())
    }
}

impl AttachmentStorage for FilesystemAttachmentStorage {
    async fn put(&self, id: Uuid, bytes: &[u8]) -> Result<(), String> {
        let dir = self.dir.clone();
        let path = self.path_for(id);
        let owned = bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
            std::fs::write(&path, &owned).map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| err.to_string())?
    }

    async fn get(&self, id: Uuid) -> Result<Vec<u8>, String> {
        let path = self.path_for(id);
        tokio::task::spawn_blocking(move || std::fs::read(&path).map_err(|err| err.to_string()))
            .await
            .map_err(|err| err.to_string())?
    }

    async fn delete(&self, id: Uuid) -> Result<(), String> {
        let path = self.path_for(id);
        tokio::task::spawn_blocking(move || match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Already gone is the steady state for orphan cleanup callers.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.to_string()),
        })
        .await
        .map_err(|err| err.to_string())?
    }
}

/// Upload validated bytes: membership-checked, allowlisted, capped, then
/// stored (bytes + row). `NotFound` covers non-members (no oracle).
///
/// # Errors
///
/// Returns [`AttachmentError::TooLarge`] over the cap,
/// [`AttachmentError::UnsupportedMime`] outside the allowlist,
/// [`AttachmentError::NotFound`] for non-members,
/// [`AttachmentError::InvalidInput`] for empty bytes/filenames, or
/// [`AttachmentError::Database`]/[`AttachmentError::Storage`] on backend
/// failure.
pub async fn upload_attachment<S: AttachmentStorage>(
    pool: &sqlx::PgPool,
    storage: &S,
    uploader_id: Uuid,
    conversation_id: Uuid,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<Attachment, AttachmentError> {
    if bytes.is_empty() {
        return Err(AttachmentError::InvalidInput(
            "attachment must not be empty".to_owned(),
        ));
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentError::TooLarge);
    }
    let mime = mime.trim().to_ascii_lowercase();
    if mime.is_empty() || mime.len() > 127 {
        return Err(AttachmentError::InvalidInput(
            "mime must not be empty".to_owned(),
        ));
    }
    if !is_allowed_mime(&mime) {
        return Err(AttachmentError::UnsupportedMime);
    }
    let filename = sanitize_filename(filename)?;
    if !crate::messaging::is_member(pool, conversation_id, uploader_id).await? {
        return Err(AttachmentError::NotFound);
    }

    let id = Uuid::now_v7();
    let digest = Sha256::digest(bytes);
    let sha256 = digest.to_vec();
    let size_bytes = i64::try_from(bytes.len())
        .map_err(|_| AttachmentError::InvalidInput("attachment too large".to_owned()))?;

    // Bytes first: a DB failure then only orphans a content-addressed file,
    // never a row pointing at missing bytes.
    storage
        .put(id, bytes)
        .await
        .map_err(AttachmentError::Storage)?;
    let insert: Result<(DateTime<Utc>,), sqlx::Error> = sqlx::query_as(
        r"INSERT INTO attachments (id, conversation_id, uploader_id, filename, mime, size_bytes, sha256)
          VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING created_at",
    )
    .bind(id)
    .bind(conversation_id)
    .bind(uploader_id)
    .bind(&filename)
    .bind(&mime)
    .bind(size_bytes)
    .bind(&sha256)
    .fetch_one(pool)
    .await;
    let row = match insert {
        Ok(row) => row,
        Err(err) => {
            // Best-effort orphan cleanup through the configured store (never
            // a hardcoded path): DB failures must not leak stored bytes.
            if let Err(cleanup) = storage.delete(id).await {
                tracing::warn!("attachment orphan cleanup failed for {id}: {cleanup}");
            }
            return Err(AttachmentError::Database(err));
        }
    };

    Ok(Attachment {
        id,
        conversation_id,
        uploader_id,
        filename,
        mime,
        size_bytes,
        sha256,
        created_at: row.0,
    })
}

/// Full attachment row as returned by the lookup query.
type AttachmentRow = (
    Uuid,
    Uuid,
    Uuid,
    String,
    String,
    i64,
    Vec<u8>,
    DateTime<Utc>,
);

/// Load an attachment's row plus its original bytes. Membership is checked
/// via the parent conversation: non-members get [`AttachmentError::NotFound`],
/// the same signal as a missing id (no oracle).
///
/// # Errors
///
/// Returns [`AttachmentError::NotFound`] for missing rows or non-members, or
/// [`AttachmentError::Database`]/[`AttachmentError::Storage`] on backend
/// failure.
pub async fn download_attachment<S: AttachmentStorage>(
    pool: &sqlx::PgPool,
    storage: &S,
    user_id: Uuid,
    attachment_id: Uuid,
) -> Result<(Attachment, Vec<u8>), AttachmentError> {
    let row: Option<AttachmentRow> = sqlx::query_as(
        r"SELECT id, conversation_id, uploader_id, filename, mime, size_bytes, sha256, created_at
           FROM attachments WHERE id = $1",
    )
    .bind(attachment_id)
    .fetch_optional(pool)
    .await
    .map_err(AttachmentError::Database)?;
    let Some(row) = row else {
        return Err(AttachmentError::NotFound);
    };
    let attachment = Attachment {
        id: row.0,
        conversation_id: row.1,
        uploader_id: row.2,
        filename: row.3,
        mime: row.4,
        size_bytes: row.5,
        sha256: row.6,
        created_at: row.7,
    };
    if !crate::messaging::is_member(pool, attachment.conversation_id, user_id).await? {
        return Err(AttachmentError::NotFound);
    }
    let bytes = storage
        .get(attachment.id)
        .await
        .map_err(AttachmentError::Storage)?;
    Ok((attachment, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_allowlist_covers_images_pdf_zip_audio_video() {
        for mime in [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "application/pdf",
            "application/zip",
            "audio/mpeg",
            "audio/ogg",
            "video/mp4",
            "video/webm",
        ] {
            assert!(is_allowed_mime(mime), "{mime} must be allowed");
        }
        for mime in [
            "text/plain",
            "text/html",
            "application/json",
            "application/octet-stream",
            "application/x-sh",
            "",
        ] {
            assert!(!is_allowed_mime(mime), "{mime} must be rejected");
        }
    }

    #[test]
    fn filename_sanitizes_separators_and_rejects_empties() {
        assert_eq!(sanitize_filename("photo.png").expect("plain"), "photo.png");
        assert_eq!(
            sanitize_filename("../photo.png").expect("traversal"),
            ".._photo.png"
        );
        assert!(sanitize_filename("").is_err());
        assert!(sanitize_filename("   ").is_err());
    }
}
