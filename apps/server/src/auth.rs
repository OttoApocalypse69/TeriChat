//! Milestone B authentication: account repo, device registry, opaque sessions.
//!
//! Secrets discipline: passwords exist only as Argon2id PHC strings
//! ([`crate::password`]); bearer tokens are returned once at login and stored
//! only as `SHA-256` hashes. Nothing here logs credential material.
//!
//! [`AuthError::InvalidCredentials`] covers both unknown handles and wrong
//! passwords so callers cannot enumerate accounts. Unknown-handle logins pay
//! for a discarded hash so their timing matches the wrong-password path.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::password::{self, PasswordError};

/// Session lifetime for freshly issued bearer tokens.
pub const SESSION_TTL_DAYS: i64 = 30;

/// Typed authentication/identity error.
#[derive(Debug)]
pub enum AuthError {
    /// `handle` already taken (after normalization).
    HandleTaken,
    /// `email` already taken (after normalization).
    EmailTaken,
    /// App-level validation rejected a value before it reached the database.
    InvalidInput(String),
    /// Unknown handle or wrong password. Deliberately one variant.
    InvalidCredentials,
    /// Missing, malformed, expired, or revoked bearer token.
    InvalidToken,
    /// System randomness unavailable while minting a token.
    Randomness(rand_core::Error),
    /// Hashing backend failure.
    Hash(PasswordError),
    /// Database failure (logged by the HTTP layer, never shown verbatim).
    Database(sqlx::Error),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HandleTaken => write!(f, "handle is already taken"),
            Self::EmailTaken => write!(f, "email is already registered"),
            Self::InvalidInput(detail) => write!(f, "invalid input: {detail}"),
            Self::InvalidCredentials => write!(f, "invalid handle or password"),
            Self::InvalidToken => write!(f, "invalid or expired session token"),
            Self::Randomness(_) => write!(f, "could not mint session token"),
            Self::Hash(_) => write!(f, "password hashing failed"),
            Self::Database(_) => write!(f, "database error"),
        }
    }
}

impl std::error::Error for AuthError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Randomness(err) => Some(err),
            Self::Hash(err) => Some(err),
            Self::Database(err) => Some(err),
            _ => None,
        }
    }
}

/// Public account view. Never carries credential material.
#[derive(Debug, Clone)]
pub struct User {
    /// Immutable `UUIDv7` account id. Handles are NOT keys.
    pub id: Uuid,
    /// Normalized (trimmed, lowercased) globally unique handle.
    pub handle: String,
    /// Normalized (trimmed, lowercased) verified-to-be-collected email.
    pub email: String,
    /// Free-form non-unique display name.
    pub display_name: String,
    /// Account creation time.
    pub created_at: DateTime<Utc>,
}

/// Registered installation. Each installation is a distinct member with its
/// own identity key; rotation appends a row, revocation stamps one.
#[derive(Debug, Clone)]
pub struct Device {
    /// Device id (`UUIDv7`).
    pub id: Uuid,
    /// Owning account.
    pub user_id: Uuid,
    /// Human label (`""` when unset).
    pub label: String,
    /// `Ed25519` identity key (always present).
    pub identity_pubkey: Vec<u8>,
    /// `X25519` agreement key for sealed envelopes. `None` for rows written
    /// before the agreement-key migration.
    pub agreement_pubkey: Option<Vec<u8>>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Live session plus the single-use bearer token. The token is shown once.
#[derive(Debug)]
pub struct NewSession {
    /// Stored session row.
    pub session: Session,
    /// Opaque bearer token (base64url, 32 random bytes). Shown once.
    pub token: String,
}

/// Stored session row (no token material).
#[derive(Debug, Clone)]
pub struct Session {
    /// Session id (`UUIDv7`).
    pub id: Uuid,
    /// Owning account.
    pub user_id: Uuid,
    /// Bound installation, if any. Unbound in Alpha; device login (Milestone
    /// C) binds it, so the field stays even though nothing reads it yet.
    #[allow(dead_code)]
    pub device_id: Option<Uuid>,
    /// Hard expiry. Revocation is separate (`revoked_at`).
    pub expires_at: DateTime<Utc>,
}

/// Authenticated request context produced by the bearer extractor.
#[derive(Debug, Clone)]
pub struct AuthSession {
    /// Requesting account.
    pub user_id: Uuid,
    /// Requesting account handle (for logs; not a key).
    pub handle: String,
    /// Session the token belongs to.
    pub session_id: Uuid,
    /// Bound installation, if any. Unbound in Alpha; device login (Milestone
    /// C) binds it, so the field stays even though nothing reads it yet.
    #[allow(dead_code)]
    pub device_id: Option<Uuid>,
}

/// Normalize a handle: trim + lowercase. Length mirrors the DB `CHECK`.
pub fn normalize_handle(raw: &str) -> Result<String, AuthError> {
    let handle = raw.trim().to_lowercase();
    if !(2..=32).contains(&handle.len()) {
        return Err(AuthError::InvalidInput(
            "handle must be 2-32 characters".to_owned(),
        ));
    }
    Ok(handle)
}

/// Normalize an email: trim + lowercase with a minimal shape check.
pub fn normalize_email(raw: &str) -> Result<String, AuthError> {
    let email = raw.trim().to_lowercase();
    let well_formed = email.contains('@')
        && email
            .split('@')
            .nth(1)
            .is_some_and(|domain| domain.contains('.'));
    if !well_formed {
        return Err(AuthError::InvalidInput(
            "email must look like name@domain.tld".to_owned(),
        ));
    }
    Ok(email)
}

/// Normalize a display name: trim, non-empty, length mirrors the DB `CHECK`.
pub fn normalize_display_name(raw: &str) -> Result<String, AuthError> {
    let name = raw.trim().to_owned();
    if name.is_empty() || name.len() > 64 {
        return Err(AuthError::InvalidInput(
            "display name must be 1-64 characters".to_owned(),
        ));
    }
    Ok(name)
}

/// Create an account: validate, Argon2id-hash, insert. Maps unique
/// violations to [`AuthError::HandleTaken`]/[`AuthError::EmailTaken`].
pub async fn create_user(
    pool: &sqlx::PgPool,
    handle: &str,
    email: &str,
    display_name: &str,
    password: &str,
) -> Result<User, AuthError> {
    let handle = normalize_handle(handle)?;
    let email = normalize_email(email)?;
    let display_name = normalize_display_name(display_name)?;
    if password.is_empty() {
        return Err(AuthError::InvalidInput(
            "password must not be empty".to_owned(),
        ));
    }
    let password_hash = password::hash_password(password).map_err(AuthError::Hash)?;

    let row: (Uuid, String, String, String, DateTime<Utc>) = sqlx::query_as(
        r"INSERT INTO users (id, handle, email, display_name, password_hash)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, handle, email, display_name, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(&handle)
    .bind(&email)
    .bind(&display_name)
    .bind(&password_hash)
    .fetch_one(pool)
    .await
    .map_err(map_constraint_violation)?;

    Ok(User {
        id: row.0,
        handle: row.1,
        email: row.2,
        display_name: row.3,
        created_at: row.4,
    })
}

/// Resolve a handle to its account id. Unknown handles are a plain input
/// error (peer lookup, not login — no oracle concern beyond the 400 itself).
pub async fn user_id_by_handle(pool: &sqlx::PgPool, handle: &str) -> Result<Uuid, AuthError> {
    let handle = normalize_handle(handle)?;
    let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE handle = $1")
        .bind(&handle)
        .fetch_optional(pool)
        .await
        .map_err(AuthError::Database)?;
    id.ok_or(AuthError::InvalidInput("unknown peer handle".to_owned()))
}

/// Raw device row shared by registration reads.
type DeviceRow = (Uuid, Uuid, String, Vec<u8>, Option<Vec<u8>>, DateTime<Utc>);

/// Register an installation for an account with its identity public key
/// (`Ed25519`, 32 bytes) and agreement public key (`X25519`, 32 bytes).
/// Degenerate keys (all-zero, unparsable identity) are refused: a planted
/// zero agreement key would make every envelope to that device readable by
/// whoever planted it (see `crates/tericrypt`).
pub async fn register_device(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    label: &str,
    identity_pubkey: [u8; 32],
    agreement_pubkey: [u8; 32],
) -> Result<Device, AuthError> {
    if !tericrypt::valid_verify_key(&identity_pubkey)
        || !tericrypt::valid_agreement_key(&agreement_pubkey)
    {
        return Err(AuthError::InvalidInput(
            "device keys are not valid public keys".to_owned(),
        ));
    }
    let row: DeviceRow = sqlx::query_as(
        r"INSERT INTO devices (id, user_id, label, identity_pubkey, agreement_pubkey)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, user_id, label, identity_pubkey, agreement_pubkey, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(label.trim())
    .bind(identity_pubkey.as_slice())
    .bind(agreement_pubkey.as_slice())
    .fetch_one(pool)
    .await
    .map_err(AuthError::Database)?;

    Ok(Device {
        id: row.0,
        user_id: row.1,
        label: row.2,
        identity_pubkey: row.3,
        agreement_pubkey: row.4,
        created_at: row.5,
    })
}

/// Log in: resolve handle, verify password, mint an opaque session.
/// Unknown handles cost one discarded hash so their timing matches the
/// wrong-password path (no enumeration oracle beyond network timing noise).
pub async fn login(
    pool: &sqlx::PgPool,
    handle: &str,
    password: &str,
) -> Result<NewSession, AuthError> {
    let handle = normalize_handle(handle)?;
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE handle = $1")
            .bind(&handle)
            .fetch_optional(pool)
            .await
            .map_err(AuthError::Database)?;

    let Some((user_id, password_hash)) = row else {
        let _ = password::hash_password(password).map_err(AuthError::Hash)?;
        return Err(AuthError::InvalidCredentials);
    };
    if !password::verify_password(&password_hash, password) {
        return Err(AuthError::InvalidCredentials);
    }

    let mut raw = [0_u8; 32];
    OsRng
        .try_fill_bytes(&mut raw)
        .map_err(AuthError::Randomness)?;
    let token = URL_SAFE_NO_PAD.encode(raw);
    let token_hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let expires_at = Utc::now() + Duration::days(SESSION_TTL_DAYS);

    let session_row: (Uuid, Uuid, Option<Uuid>, DateTime<Utc>) = sqlx::query_as(
        r"INSERT INTO sessions (id, user_id, token_hash, expires_at)
           VALUES ($1, $2, $3, $4)
           RETURNING id, user_id, device_id, expires_at",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(token_hash.as_slice())
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .map_err(AuthError::Database)?;

    Ok(NewSession {
        session: Session {
            id: session_row.0,
            user_id: session_row.1,
            device_id: session_row.2,
            expires_at: session_row.3,
        },
        token,
    })
}

/// Log out: stamp revocation. Idempotent — unknown or already-revoked tokens
/// still return `Ok` so logout responses reveal nothing.
pub async fn logout(pool: &sqlx::PgPool, token: &str) -> Result<(), AuthError> {
    let token_hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    sqlx::query(
        r"UPDATE sessions SET revoked_at = now()
           WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash.as_slice())
    .execute(pool)
    .await
    .map_err(AuthError::Database)?;
    Ok(())
}

/// Resolve a bearer token to its session. Rejects unknown, revoked, and
/// expired tokens identically as [`AuthError::InvalidToken`].
pub async fn authenticate(pool: &sqlx::PgPool, token: &str) -> Result<AuthSession, AuthError> {
    let token_hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let row: Option<(Uuid, Uuid, Option<Uuid>, String)> = sqlx::query_as(
        r"SELECT s.id, s.user_id, s.device_id, u.handle
           FROM sessions s JOIN users u ON u.id = s.user_id
           WHERE s.token_hash = $1
             AND s.revoked_at IS NULL
             AND s.expires_at > now()",
    )
    .bind(token_hash.as_slice())
    .fetch_optional(pool)
    .await
    .map_err(AuthError::Database)?;

    row.map(|found| AuthSession {
        session_id: found.0,
        user_id: found.1,
        device_id: found.2,
        handle: found.3,
    })
    .ok_or(AuthError::InvalidToken)
}

/// Map Postgres constraint violations to typed auth errors; anything else
/// stays a database error.
fn map_constraint_violation(err: sqlx::Error) -> AuthError {
    if let sqlx::Error::Database(db_err) = &err {
        if db_err.code().as_deref() == Some("23505") {
            let constraint = db_err.constraint().unwrap_or_default();
            if constraint.contains("handle") {
                return AuthError::HandleTaken;
            }
            if constraint.contains("email") {
                return AuthError::EmailTaken;
            }
        }
        if db_err.code().as_deref() == Some("23514") {
            return AuthError::InvalidInput("value rejected by database constraints".to_owned());
        }
    }
    AuthError::Database(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_normalization() {
        assert_eq!(normalize_handle("  Teri  ").unwrap(), "teri");
        assert!(normalize_handle("a").is_err());
        assert!(normalize_handle(&"x".repeat(33)).is_err());
    }

    #[test]
    fn email_normalization() {
        assert_eq!(
            normalize_email("  Teri@Example.COM ").unwrap(),
            "teri@example.com"
        );
        assert!(normalize_email("not-an-email").is_err());
        assert!(normalize_email("missing@tld").is_err());
    }

    #[test]
    fn display_name_normalization() {
        assert_eq!(normalize_display_name("  Teri  ").unwrap(), "Teri");
        assert!(normalize_display_name("   ").is_err());
        assert!(normalize_display_name(&"x".repeat(65)).is_err());
    }

    /// Requires a live database (`DATABASE_URL=... cargo test`); skips
    /// honestly without one. Exercises the full account lifecycle.
    #[tokio::test]
    async fn account_lifecycle() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: account_lifecycle (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let handle = format!("bro{stamp}");
        let email = format!("bro{stamp}@example.com");

        let user = create_user(&pool, &handle, &email, "Bro", "s3cret-pw")
            .await
            .expect("register user");
        assert_eq!(user.handle, handle.to_lowercase());

        // Duplicates, each mapping to its typed error.
        let taken = create_user(&pool, &handle, "other@example.com", "X", "pw12")
            .await
            .unwrap_err();
        assert!(matches!(taken, AuthError::HandleTaken), "got {taken:?}");
        let taken = create_user(&pool, "otherhandle", &email, "X", "pw12")
            .await
            .unwrap_err();
        assert!(matches!(taken, AuthError::EmailTaken), "got {taken:?}");

        // Wrong password and unknown handle are indistinguishable.
        let wrong = login(&pool, &handle, "nope").await.unwrap_err();
        let unknown = login(&pool, "nobody-here", "nope").await.unwrap_err();
        assert!(matches!(wrong, AuthError::InvalidCredentials));
        assert!(matches!(unknown, AuthError::InvalidCredentials));

        let issued = login(&pool, &handle, "s3cret-pw").await.expect("login");
        assert!(!issued.token.is_empty());
        let ctx = authenticate(&pool, &issued.token)
            .await
            .expect("bearer resolves");
        assert_eq!(ctx.user_id, user.id);
        assert_eq!(ctx.session_id, issued.session.id);

        logout(&pool, &issued.token).await.expect("logout");
        // Idempotent second logout, then the token is dead.
        logout(&pool, &issued.token).await.expect("logout again");
        let dead = authenticate(&pool, &issued.token).await.unwrap_err();
        assert!(matches!(dead, AuthError::InvalidToken));
        assert!(authenticate(&pool, "garbage-token").await.is_err());

        pool.close().await;
    }

    /// Requires a live database; skips honestly without one.
    #[tokio::test]
    async fn device_registration() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: device_registration (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = create_user(
            &pool,
            &format!("dev{stamp}"),
            &format!("dev{stamp}@example.com"),
            "Dev",
            "pw-for-dev",
        )
        .await
        .expect("register user");

        let device_keys = tericrypt::IdentityKeypair::generate().expect("device keys");
        let device = register_device(
            &pool,
            user.id,
            "laptop",
            device_keys.identity_verify_key(),
            device_keys.agreement_pubkey(),
        )
        .await
        .expect("register device");
        assert_eq!(device.user_id, user.id);
        assert_eq!(device.label, "laptop");
        assert_eq!(
            device.agreement_pubkey,
            Some(device_keys.agreement_pubkey().to_vec())
        );

        // Degenerate keys are refused, never stored.
        assert!(register_device(
            &pool,
            user.id,
            "zero-id",
            [0_u8; 32],
            device_keys.agreement_pubkey()
        )
        .await
        .is_err());
        assert!(register_device(
            &pool,
            user.id,
            "zero-agree",
            device_keys.identity_verify_key(),
            [0_u8; 32]
        )
        .await
        .is_err());

        pool.close().await;
    }
}
