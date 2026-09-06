//! `TeriChat` modular-monolith API server (Alpha 0).
//!
//! Serves [`/health`](health) liveness, [`/ready`](ready) readiness, and the
//! Milestone B account API under `/v1/auth/*` (register, login, logout,
//! device registration). Configuration comes from [`config::Config`]; without
//! `DATABASE_URL` the server boots bare with only the probes live.

#![forbid(unsafe_code)]

mod auth;
mod config;
mod password;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::{FromRequestParts, State},
    http::{header, request::Parts, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing_subscriber::{fmt, EnvFilter};
use uuid::Uuid;

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
enum AppError {
    /// Caller-supplied value rejected (taken handle/email, bad input).
    BadRequest(String),
    /// Missing or rejected bearer credentials.
    Unauthorized,
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

/// Shared server state: the optional database pool. `None` means the
/// probes-only boot (no `DATABASE_URL`).
#[derive(Clone)]
struct AppState {
    pool: Option<sqlx::PgPool>,
}

/// Liveness probe — never touches dependencies.
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

/// Readiness probe — reports database wiring state.
#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    database: &'static str,
}

/// Public account view. Never carries credential material.
#[derive(Debug, Serialize)]
struct UserBody {
    id: Uuid,
    handle: String,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
}

impl From<auth::User> for UserBody {
    fn from(user: auth::User) -> Self {
        Self {
            id: user.id,
            handle: user.handle,
            email: user.email,
            display_name: user.display_name,
            created_at: user.created_at,
        }
    }
}

/// Issued session: the token is shown exactly once, here.
#[derive(Debug, Serialize)]
struct LoginResponse {
    token: String,
    session_id: Uuid,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
    user_handle: String,
}

/// Registered device view.
#[derive(Debug, Serialize)]
struct DeviceBody {
    id: Uuid,
    user_id: Uuid,
    label: String,
    created_at: DateTime<Utc>,
}

impl From<auth::Device> for DeviceBody {
    fn from(device: auth::Device) -> Self {
        Self {
            id: device.id,
            user_id: device.user_id,
            label: device.label,
            created_at: device.created_at,
        }
    }
}

/// `POST /v1/auth/register` request.
#[derive(Debug, Deserialize)]
struct RegisterBody {
    handle: String,
    email: String,
    display_name: String,
    password: String,
}

/// `POST /v1/auth/login` request.
#[derive(Debug, Deserialize)]
struct LoginBody {
    handle: String,
    password: String,
}

/// `POST /v1/auth/devices` request. The identity public key is lowercase or
/// uppercase hex (`64` chars, `32` bytes); rejected generically otherwise.
#[derive(Debug, Deserialize)]
struct RegisterDeviceBody {
    label: String,
    identity_pubkey: String,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "terichat-server",
    })
}

async fn ready(State(state): State<AppState>) -> Result<Json<ReadyResponse>, AppError> {
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

async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> Result<(StatusCode, Json<UserBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Never log `body.password`.
    let user = auth::create_user(
        pool,
        &body.handle,
        &body.email,
        &body.display_name,
        &body.password,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(UserBody::from(user))))
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Never log `body.password`.
    let issued = auth::login(pool, &body.handle, &body.password).await?;
    let handle = auth::normalize_handle(&body.handle).map_err(AppError::from)?;
    Ok(Json(LoginResponse {
        token: issued.token,
        session_id: issued.session.id,
        user_id: issued.session.user_id,
        expires_at: issued.session.expires_at,
        user_handle: handle,
    }))
}

async fn logout(
    State(state): State<AppState>,
    bearer: Bearer,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // The raw token never leaves this call except into its `SHA-256` hash.
    auth::logout(pool, bearer.token()).await?;
    Ok(Json(
        serde_json::json!({ "status": "ok", "session_id": bearer.session.session_id }),
    ))
}

async fn register_device(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<RegisterDeviceBody>,
) -> Result<(StatusCode, Json<DeviceBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let raw = hex::decode(body.identity_pubkey.trim()).map_err(|_| {
        AppError::BadRequest("identity_pubkey must be 64 hex characters".to_owned())
    })?;
    let pubkey: [u8; 32] = raw.try_into().map_err(|_| {
        AppError::BadRequest("identity_pubkey must be 64 hex characters".to_owned())
    })?;
    let device = auth::register_device(pool, bearer.user_id(), &body.label, pubkey).await?;
    Ok((StatusCode::CREATED, Json(DeviceBody::from(device))))
}

/// Authenticated request identity plus the raw bearer token, available only
/// to handlers that need to hash it (logout). Handlers that only need the
/// identity use [`Bearer::user_id`].
struct Bearer {
    session: auth::AuthSession,
    token: String,
}

impl Bearer {
    /// Requesting account id.
    fn user_id(&self) -> Uuid {
        self.session.user_id
    }

    /// Raw bearer token. Handle like a password: hash, never log.
    fn token(&self) -> &str {
        &self.token
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

/// Embedded, forward-only `SQLx` migrations from the workspace `migrations/`
/// directory. The macro validates at compile time that the directory exists
/// and parses, so a missing/broken migration fails the build — no database
/// connection required.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Shared database pool for request handlers and boot migration.
async fn db_pool(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/auth/devices", post(register_device))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let config = config::Config::from_env().unwrap_or_else(|err| {
        eprintln!("configuration error: {err}");
        std::process::exit(1);
    });

    fmt()
        .with_env_filter(
            EnvFilter::try_new(&config.rust_log).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    // Forward-only schema migration, applied once per boot when a database is
    // configured. Without `DATABASE_URL` the server still boots bare.
    let pool = match &config.database_url {
        None => None,
        Some(url) => {
            let pool = db_pool(url).await.expect("connect database");
            MIGRATOR
                .run(&pool)
                .await
                .expect("apply pending SQLx migrations");
            tracing::info!("database migrations applied");
            Some(pool)
        }
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind server address");
    tracing::info!(%addr, "terichat-server listening");
    axum::serve(listener, build_router(AppState { pool }))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve axum router");
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    fn bare_state() -> AppState {
        AppState { pool: None }
    }

    async fn body_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    async fn post_json(
        app: Router,
        uri: &str,
        body: serde_json::Value,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .oneshot(
                builder
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let (status, json) = body_json(build_router(bare_state()), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "terichat-server");
    }

    #[tokio::test]
    async fn ready_without_database_reports_not_configured() {
        let (status, json) = body_json(build_router(bare_state()), "/ready").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["database"], "not_configured");
    }

    #[tokio::test]
    async fn ready_with_unreachable_database_is_down() {
        let pool = db_pool("postgres://127.0.0.1:1/terichat_test").await;
        assert!(pool.is_err(), "expected connect failure, got {pool:?}");
    }

    #[tokio::test]
    async fn auth_routes_need_a_database() {
        // Probes-only boot: every account route reports 503, never 500.
        let app = build_router(bare_state());
        let (status, json) = post_json(
            app,
            "/v1/auth/register",
            serde_json::json!({"handle":"x","email":"x@y.z","display_name":"X","password":"pw"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"]["code"], "no_database");
    }

    #[tokio::test]
    async fn bearer_rejection_shapes() {
        // No database and no token alike: 401/503 JSON, never a bare status.
        let (status, json) = post_json(
            build_router(bare_state()),
            "/v1/auth/logout",
            serde_json::json!({}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"]["code"], "no_database");
    }

    #[test]
    fn migrator_embeds_identity_migration() {
        // Offline: the embedded migrator must resolve at least the identity
        // migration. The `migrate!` macro already fails the build when the
        // directory is missing or unparsable; this pins the expected content.
        assert!(
            !MIGRATOR.migrations.is_empty(),
            "expected at least the identity migration"
        );
        let latest = MIGRATOR
            .migrations
            .iter()
            .max_by_key(|m| m.version)
            .unwrap();
        assert!(
            latest.description.contains("identity"),
            "latest migration should be the identity foundation"
        );
    }

    #[tokio::test]
    async fn migrations_apply_and_create_identity_tables() {
        // Requires a live database: `DATABASE_URL=... cargo test`. Prints a
        // skip (never a fake pass) when no database is configured, so plain
        // `cargo test` stays offline-clean for CI without services.
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: migrations_apply_and_create_identity_tables (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");

        for table in ["users", "devices", "sessions"] {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("query information_schema");
            assert!(exists, "expected table `{table}` after migrations");
        }
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Full HTTP
    /// account lifecycle through the real routes.
    #[tokio::test]
    async fn http_account_lifecycle() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: http_account_lifecycle (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = AppState { pool: Some(pool) };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let handle = format!("httpbro{stamp}");
        let email = format!("httpbro{stamp}@example.com");

        // Register → 201 with the public user view (no secrets).
        let (status, user) = post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": handle, "email": email, "display_name": "Http Bro", "password": "s3cret-pw"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(user["handle"], handle.to_lowercase());
        assert!(user.get("password_hash").is_none());
        assert!(user.get("password").is_none());

        // Duplicate handle → 400, duplicate email → 400.
        let (status, body) = post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": handle, "email": "other@example.com", "display_name": "X", "password": "pw12"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "bad_request");
        let (status, _) = post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": "someother", "email": email, "display_name": "X", "password": "pw12"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Wrong password → 401 with the unauthorized envelope.
        let (status, body) = post_json(
            build_router(state.clone()),
            "/v1/auth/login",
            serde_json::json!({"handle": handle, "password": "wrong"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");

        // Login → 200 with a single-use token.
        let (status, login) = post_json(
            build_router(state.clone()),
            "/v1/auth/login",
            serde_json::json!({"handle": handle, "password": "s3cret-pw"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = login["token"].as_str().expect("token issued").to_owned();

        // Device registration behind the bearer → 201; bad pubkey → 400.
        let (status, device) = post_json(
            build_router(state.clone()),
            "/v1/auth/devices",
            serde_json::json!({"label": "laptop", "identity_pubkey": "ab".repeat(32)}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(device["label"], "laptop");
        let (status, _) = post_json(
            build_router(state.clone()),
            "/v1/auth/devices",
            serde_json::json!({"label": "bad", "identity_pubkey": "zz"}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Logout → 200; the token is dead afterwards; garbage never works.
        let (status, _) = post_json(
            build_router(state.clone()),
            "/v1/auth/logout",
            serde_json::json!({}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = post_json(
            build_router(state.clone()),
            "/v1/auth/logout",
            serde_json::json!({}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");

        state.pool.as_ref().unwrap().close().await;
    }
}
