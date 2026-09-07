//! `TeriChat` modular-monolith API server (Alpha 0).
//!
//! Serves [`/health`](health) liveness, [`/ready`](ready) readiness, and the
//! Milestone B account API under `/v1/auth/*` (register, login, logout,
//! device registration). Configuration comes from [`config::Config`]; without
//! `DATABASE_URL` the server boots bare with only the probes live.

#![forbid(unsafe_code)]

mod auth;
mod config;
mod gateway;
mod messaging;
mod password;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::{FromRequestParts, Query, State},
    http::{header, request::Parts, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
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
    /// Caller is not a conversation member (also covers missing rows).
    Forbidden,
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

/// Shared server state: the optional database pool plus the realtime fan-out
/// hub. `pool: None` means the probes-only boot (no `DATABASE_URL`).
#[derive(Clone)]
struct AppState {
    pool: Option<sqlx::PgPool>,
    hub: broadcast::Sender<messaging::OutboxEntry>,
}

/// Hub capacity: live burst buffer. Overflow drops to resume (`Lagged`
/// receivers re-anchor from the database), never to data loss.
const HUB_CAPACITY: usize = 1024;

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

/// Registered device view, with both public keys as hex (`None` agreement
/// key for rows written before the agreement-key migration).
#[derive(Debug, Serialize)]
struct DeviceBody {
    id: Uuid,
    user_id: Uuid,
    label: String,
    identity_pubkey: String,
    agreement_pubkey: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<auth::Device> for DeviceBody {
    fn from(device: auth::Device) -> Self {
        Self {
            id: device.id,
            user_id: device.user_id,
            label: device.label,
            identity_pubkey: hex::encode(&device.identity_pubkey),
            agreement_pubkey: device.agreement_pubkey.as_deref().map(hex::encode),
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

/// `POST /v1/auth/devices` request. Both keys are lowercase or uppercase hex
/// (`64` chars, `32` bytes each); rejected generically otherwise.
#[derive(Debug, Deserialize)]
struct RegisterDeviceBody {
    label: String,
    identity_pubkey: String,
    agreement_pubkey: String,
}

/// `POST /v1/conversations/dm` request: open (or reopen) the DM with a peer.
#[derive(Debug, Deserialize)]
struct DmBody {
    peer_handle: String,
}

/// `POST /v1/conversations` request: start a group conversation.
#[derive(Debug, Deserialize)]
struct GroupBody {
    member_handles: Vec<String>,
}

/// Conversation view with member account ids.
#[derive(Debug, Serialize)]
struct ConversationBody {
    id: Uuid,
    kind: String,
    members: Vec<Uuid>,
}

impl From<messaging::Conversation> for ConversationBody {
    fn from(conversation: messaging::Conversation) -> Self {
        Self {
            id: conversation.id,
            kind: conversation.kind,
            members: conversation.members,
        }
    }
}

/// `POST /v1/messages` request. Envelope bytes travel base64-encoded;
/// the server never decodes them into anything but opaque storage.
#[derive(Debug, Deserialize)]
struct SendBody {
    conversation_id: Uuid,
    client_msg_id: Uuid,
    ciphertext_b64: String,
    nonce_b64: Option<String>,
}

/// Stored message view. `deduped` reports an idempotent retry.
#[derive(Debug, Serialize)]
struct MessageBody {
    id: Uuid,
    conversation_id: Uuid,
    sender_id: Uuid,
    seq: i64,
    ciphertext_b64: String,
    nonce_b64: Option<String>,
    client_msg_id: Uuid,
    sent_at: DateTime<Utc>,
    deduped: bool,
}

impl MessageBody {
    fn new(message: &messaging::Message, deduped: bool) -> Self {
        Self {
            id: message.id,
            conversation_id: message.conversation_id,
            sender_id: message.sender_id,
            seq: message.seq,
            ciphertext_b64: STANDARD.encode(&message.ciphertext),
            nonce_b64: message.nonce.as_deref().map(|bytes| STANDARD.encode(bytes)),
            client_msg_id: message.client_msg_id,
            sent_at: message.sent_at,
            deduped,
        }
    }
}

/// `GET /v1/messages` query: history after `since_seq`, oldest first.
#[derive(Debug, Deserialize)]
struct HistoryParams {
    conversation_id: Uuid,
    since_seq: Option<i64>,
    limit: Option<i64>,
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
    let decode_key = |raw: &str| {
        hex::decode(raw.trim())
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(AppError::BadRequest(
                "identity_pubkey and agreement_pubkey must each be 64 hex characters".to_owned(),
            ))
    };
    let identity_pubkey: [u8; 32] = decode_key(&body.identity_pubkey)?;
    let agreement_pubkey: [u8; 32] = decode_key(&body.agreement_pubkey)?;
    let device = auth::register_device(
        pool,
        bearer.user_id(),
        &body.label,
        identity_pubkey,
        agreement_pubkey,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(DeviceBody::from(device))))
}

async fn create_dm(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<DmBody>,
) -> Result<(StatusCode, Json<ConversationBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let peer = auth::user_id_by_handle(pool, &body.peer_handle).await?;
    let conversation = messaging::find_or_create_dm(pool, bearer.user_id(), peer).await?;
    Ok((
        StatusCode::CREATED,
        Json(ConversationBody::from(conversation)),
    ))
}

async fn create_group(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<GroupBody>,
) -> Result<(StatusCode, Json<ConversationBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    if body.member_handles.is_empty() {
        return Err(AppError::BadRequest(
            "group needs at least one member".to_owned(),
        ));
    }
    let mut members = Vec::with_capacity(body.member_handles.len());
    for handle in &body.member_handles {
        members.push(auth::user_id_by_handle(pool, handle).await?);
    }
    let conversation =
        messaging::create_conversation(pool, bearer.user_id(), "group", &members).await?;
    Ok((
        StatusCode::CREATED,
        Json(ConversationBody::from(conversation)),
    ))
}

async fn send_message(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<SendBody>,
) -> Result<(StatusCode, Json<MessageBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Envelope bytes are opaque: decoded from transport encoding straight
    // into storage, never inspected or logged.
    let ciphertext = STANDARD
        .decode(body.ciphertext_b64.trim())
        .map_err(|_| AppError::BadRequest("ciphertext_b64 is not valid base64".to_owned()))?;
    let nonce = body
        .nonce_b64
        .as_deref()
        .map(|raw| {
            STANDARD
                .decode(raw.trim())
                .map_err(|_| AppError::BadRequest("nonce_b64 is not valid base64".to_owned()))
        })
        .transpose()?;
    let (message, created) = messaging::send_message(
        pool,
        bearer.user_id(),
        body.conversation_id,
        body.client_msg_id,
        &ciphertext,
        nonce.as_deref(),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(MessageBody::new(&message, !created)),
    ))
}

async fn message_history(
    State(state): State<AppState>,
    bearer: Bearer,
    Query(params): Query<HistoryParams>,
) -> Result<Json<Vec<MessageBody>>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let messages = messaging::message_history(
        pool,
        bearer.user_id(),
        params.conversation_id,
        params.since_seq.unwrap_or(0),
        params.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(
        messages
            .iter()
            .map(|message| MessageBody::new(message, false))
            .collect(),
    ))
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
        .route("/v1/conversations/dm", post(create_dm))
        .route("/v1/conversations", post(create_group))
        .route("/v1/messages", post(send_message).get(message_history))
        .route("/v1/gateway", get(gateway::gateway_handler))
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

    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    if let Some(pool) = &pool {
        // At-least-once fan-out; the next boot re-claims anything unmarked.
        tokio::spawn(messaging::outbox_worker(pool.clone(), hub.clone()));
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind server address");
    tracing::info!(%addr, "terichat-server listening");
    axum::serve(listener, build_router(AppState { pool, hub }))
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
        let (hub, _) = broadcast::channel(HUB_CAPACITY);
        AppState { pool: None, hub }
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
        // Offline: the embedded migrator must resolve the known migrations.
        // The `migrate!` macro already fails the build when the directory is
        // missing or unparsable; this pins the expected content.
        assert!(
            !MIGRATOR.migrations.is_empty(),
            "expected at least the identity migration"
        );
        // Note: sqlx renders filename underscores as spaces in descriptions.
        for expected in ["identity", "messaging", "device agreement keys"] {
            assert!(
                MIGRATOR
                    .migrations
                    .iter()
                    .any(|migration| migration.description.contains(expected)),
                "expected a {expected} migration"
            );
        }
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
        let state = AppState {
            pool: Some(pool),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

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

    /// Requires a live database; skips honestly without one. Device keys over
    /// HTTP: real keys register (201), non-hex is rejected (400), and
    /// well-formed-but-degenerate keys are refused, never stored (400).
    #[tokio::test]
    async fn http_device_key_validation() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: http_device_key_validation (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let handle = format!("devkey{stamp}");
        post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": handle, "email": format!("{handle}@example.com"), "display_name": "Keys", "password": "pw-keys-1"}),
            None,
        )
        .await;
        let (_, login) = post_json(
            build_router(state.clone()),
            "/v1/auth/login",
            serde_json::json!({"handle": handle, "password": "pw-keys-1"}),
            None,
        )
        .await;
        let token = login["token"].as_str().expect("token issued").to_owned();

        let device_keys = tericrypt::IdentityKeypair::generate().expect("device keys");
        let identity_hex = hex::encode(device_keys.identity_verify_key());
        let agree_hex = hex::encode(device_keys.agreement_pubkey());
        let (status, device) = post_json(
            build_router(state.clone()),
            "/v1/auth/devices",
            serde_json::json!({"label": "laptop", "identity_pubkey": identity_hex, "agreement_pubkey": agree_hex.clone()}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(device["label"], "laptop");
        assert_eq!(device["agreement_pubkey"], agree_hex);
        let zeros = "00".repeat(32);
        for (label, identity) in [("bad", "zz"), ("zero", zeros.as_str())] {
            let (status, _) = post_json(
                build_router(state.clone()),
                "/v1/auth/devices",
                serde_json::json!({"label": label, "identity_pubkey": identity, "agreement_pubkey": agree_hex}),
                Some(&token),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "label {label}");
        }
        pool.close().await;
    }

    /// Plain (non-TLS) client WebSocket used by the gateway tests.
    type WsStream = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// Register + log in two users and open their DM. Returns
    /// `(alice_token, bob_token, bob_id, dm_id)`.
    async fn ws_fixture(pool: &sqlx::PgPool, tag: &str) -> (String, String, Uuid, Uuid) {
        let alice = auth::create_user(
            pool,
            &format!("wsalice{tag}"),
            &format!("wsalice{tag}@example.com"),
            "WsAlice",
            "pw-alice-ws",
        )
        .await
        .expect("register alice");
        let bob = auth::create_user(
            pool,
            &format!("wsbob{tag}"),
            &format!("wsbob{tag}@example.com"),
            "WsBob",
            "pw-bob-ws",
        )
        .await
        .expect("register bob");
        let alice_token = auth::login(pool, &alice.handle, "pw-alice-ws")
            .await
            .expect("login alice")
            .token;
        let bob_token = auth::login(pool, &bob.handle, "pw-bob-ws")
            .await
            .expect("login bob")
            .token;
        let dm = messaging::find_or_create_dm(pool, alice.id, bob.id)
            .await
            .expect("open dm");
        (alice_token, bob_token, bob.id, dm.id)
    }

    /// Send one message through the HTTP route (oneshot consumes routers,
    /// so each call builds a fresh one from shared state).
    async fn http_send(
        state: &AppState,
        token: &str,
        dm: Uuid,
        plaintext_b64: &str,
    ) -> (StatusCode, serde_json::Value) {
        post_json(
            build_router(state.clone()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": dm,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": plaintext_b64,
            }),
            Some(token),
        )
        .await
    }

    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    /// Send a JSON frame on a gateway test socket.
    async fn ws_send(ws: &mut WsStream, value: serde_json::Value) {
        ws.send(WsMessage::Text(value.to_string().into()))
            .await
            .expect("send ws frame");
    }

    /// Read the next text frame as JSON (15 s budget).
    async fn ws_next(ws: &mut WsStream) -> serde_json::Value {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(15), ws.next())
            .await
            .expect("ws frame in time")
            .expect("ws stream open");
        match frame.expect("ws message ok") {
            WsMessage::Text(text) => serde_json::from_str(&text).expect("json frame"),
            other => panic!("expected text frame, got {other:?}"),
        }
    }

    /// Identify and expect the next frame back.
    async fn ws_identify(ws: &mut WsStream, resume_after: Option<Uuid>) -> serde_json::Value {
        ws_send(
            ws,
            serde_json::json!({"op": "identify", "resume_after": resume_after}),
        )
        .await;
        ws_next(ws).await
    }

    /// Connect, identify, and expect `ready`. Returns the live socket.
    async fn connect_identified(
        addr: &std::net::SocketAddr,
        token: &str,
        resume_after: Option<Uuid>,
    ) -> WsStream {
        use tokio_tungstenite::connect_async;
        let (mut ws, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .expect("ws connect");
        assert_eq!(ws_identify(&mut ws, resume_after).await["op"], "ready");
        ws
    }

    /// Pull the event id out of an `event` frame.
    fn event_id_of(frame: &serde_json::Value) -> Uuid {
        frame["event"]["event_id"]
            .as_str()
            .expect("event id")
            .parse()
            .expect("event uuid")
    }

    /// Read one frame and assert it is the expected conversation event.
    /// Returns the event id for resume chaining.
    async fn expect_event(ws: &mut WsStream, seq: i64) -> Uuid {
        let frame = ws_next(ws).await;
        assert_eq!(frame["op"], "event");
        assert_eq!(frame["event"]["payload"]["data"]["seq"], seq);
        event_id_of(&frame)
    }

    /// Requires a live database; skips honestly without one. The Alpha 0
    /// milestone: two clients exchange an encrypted DM over the realtime
    /// gateway, reconnect, and resume without loss or duplicates. The first
    /// encrypted message is `bro` — opaque to the server, decoded only by
    /// this test.
    #[tokio::test]
    async fn two_clients_exchange_and_resume() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: two_clients_exchange_and_resume (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (alice_token, bob_token, bob_id, dm_id) = ws_fixture(&pool, &stamp).await;

        let (hub, _) = broadcast::channel(HUB_CAPACITY);
        let state = AppState {
            pool: Some(pool.clone()),
            hub: hub.clone(),
        };
        // HTTP posts only need the pool; each oneshot call builds its own router.
        let http_state = AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("test addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, build_router(state))
                .await
                .expect("serve test app");
        });
        let _worker = tokio::spawn(messaging::outbox_worker(pool.clone(), hub));

        let _alice_ws = connect_identified(&addr, &alice_token, None).await;
        let mut bob_ws = connect_identified(&addr, &bob_token, None).await;

        // Alice sends `bro` through the HTTP route; Bob gets the live event.
        let bro_b64 = STANDARD.encode(b"bro");
        let (status, sent) = http_send(&http_state, &alice_token, dm_id, &bro_b64).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(sent["deduped"], false);
        assert_eq!(sent["seq"], 1);

        let event1_id = expect_event(&mut bob_ws, 1).await;

        // Bob reads history: the opaque bytes decode to `bro` client-side.
        let history = messaging::message_history(&pool, bob_id, dm_id, 0, 50)
            .await
            .expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].ciphertext, b"bro");

        // Second message flows live too.
        let second_b64 = STANDARD.encode(b"second");
        let (status, _) = http_send(&http_state, &alice_token, dm_id, &second_b64).await;
        assert_eq!(status, StatusCode::CREATED);
        let event2_id = expect_event(&mut bob_ws, 2).await;
        assert_ne!(event1_id, event2_id);

        // Bob drops, Alice sends a third, Bob resumes after the second:
        // exactly one replayed event, no duplicates.
        bob_ws.close(None).await.expect("bob close");
        drop(bob_ws);
        let third_b64 = STANDARD.encode(b"third");
        let (status, _) = http_send(&http_state, &alice_token, dm_id, &third_b64).await;
        assert_eq!(status, StatusCode::CREATED);

        let mut bob_ws = connect_identified(&addr, &bob_token, Some(event2_id)).await;
        let _replayed_id = expect_event(&mut bob_ws, 3).await;
        let quiet = tokio::time::timeout(std::time::Duration::from_secs(2), bob_ws.next()).await;
        assert!(
            quiet.is_err(),
            "expected no duplicate delivery, got {quiet:?}"
        );

        server.abort();
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The Milestone D
    /// proof: a sealed `TeriCrypt` envelope travels the real send/history
    /// path as opaque bytes and only the recipient device opens it. The
    /// server never sees `bro`.
    #[tokio::test]
    async fn sealed_dm_end_to_end() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: sealed_dm_end_to_end (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (alice_token, _, bob_id, dm_id) = ws_fixture(&pool, &stamp).await;
        let alice_id = auth::user_id_by_handle(&pool, &format!("wsalice{stamp}"))
            .await
            .expect("alice id");

        // Device keypairs live client-side; only the public halves register.
        let alice_dev = tericrypt::IdentityKeypair::generate().expect("alice device keys");
        let bob_dev = tericrypt::IdentityKeypair::generate().expect("bob device keys");
        let mallory_dev = tericrypt::IdentityKeypair::generate().expect("mallory device keys");
        auth::register_device(
            &pool,
            alice_id,
            "alice-phone",
            alice_dev.identity_verify_key(),
            alice_dev.agreement_pubkey(),
        )
        .await
        .expect("register alice device");
        auth::register_device(
            &pool,
            bob_id,
            "bob-laptop",
            bob_dev.identity_verify_key(),
            bob_dev.agreement_pubkey(),
        )
        .await
        .expect("register bob device");

        // Alice seals `bro` to Bob's agreement key and sends the wire bytes.
        let envelope =
            tericrypt::seal(&alice_dev, &bob_dev.agreement_pubkey(), b"bro").expect("seal");
        let wire = envelope.to_bytes();
        assert!(wire.len() >= tericrypt::HEADER_LEN);
        let http_state = AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };
        let (status, sent) = post_json(
            build_router(http_state),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": dm_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(&wire),
            }),
            Some(&alice_token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        // Bob reads history and opens the envelope; Mallory's device cannot.
        let history = messaging::message_history(&pool, bob_id, dm_id, 0, 50)
            .await
            .expect("bob history");
        assert_eq!(history.len(), 1);
        let received =
            tericrypt::SealedEnvelope::from_bytes(&history[0].ciphertext).expect("parse envelope");
        let plaintext = tericrypt::open(&bob_dev, &alice_dev.identity_verify_key(), &received)
            .expect("bob opens");
        assert_eq!(plaintext, b"bro");
        assert!(
            tericrypt::open(&mallory_dev, &alice_dev.identity_verify_key(), &received).is_err()
        );

        // The stored bytes are the sealed envelope, not plaintext.
        assert_eq!(history[0].ciphertext, wire);
        assert!(sent["seq"].as_i64().unwrap() >= 1);
        pool.close().await;
    }
}
