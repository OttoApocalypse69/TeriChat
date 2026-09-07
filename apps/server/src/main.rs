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
mod workspaces;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::{FromRequestParts, Path, Query, State},
    http::{header, request::Parts, StatusCode},
    response::IntoResponse,
    routing::{delete, get, patch, post},
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
    /// Caller is not a workspace member or lacks workspace permission.
    /// Carries a safe message (never workspace existence details).
    Denied(String),
    /// Named object (e.g. invite) is unknown or unusable.
    NotFound(String),
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

/// Workspace view with the caller's role.
#[derive(Debug, Serialize)]
struct WorkspaceBody {
    id: Uuid,
    name: String,
    owner_id: Uuid,
    my_role: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// `POST /v1/workspaces` / `PATCH /v1/workspaces/{id}` request.
#[derive(Debug, Deserialize)]
struct WorkspaceNameBody {
    name: String,
}

/// `POST /v1/workspaces/{id}/members` request.
#[derive(Debug, Deserialize)]
struct AddMemberBody {
    user_handle: String,
    role: String,
}

/// `PATCH /v1/workspaces/{id}/members/{user_id}` request.
#[derive(Debug, Deserialize)]
struct SetRoleBody {
    role: String,
}

/// `POST /v1/workspaces/{id}/bans` request.
#[derive(Debug, Deserialize)]
struct BanBody {
    user_id: Uuid,
    reason: Option<String>,
}

/// Channel view with the backing conversation for the message path.
#[derive(Debug, Serialize)]
struct ChannelBody {
    id: Uuid,
    workspace_id: Uuid,
    conversation_id: Uuid,
    name: String,
    kind: String,
    created_by: Uuid,
    created_at: DateTime<Utc>,
}

impl From<workspaces::Channel> for ChannelBody {
    fn from(channel: workspaces::Channel) -> Self {
        Self {
            id: channel.id,
            workspace_id: channel.workspace_id,
            conversation_id: channel.conversation_id,
            name: channel.name,
            kind: channel.kind,
            created_by: channel.created_by,
            created_at: channel.created_at,
        }
    }
}

/// `POST /v1/workspaces/{id}/channels` / `PATCH /v1/channels/{id}` request.
#[derive(Debug, Deserialize)]
struct ChannelNameBody {
    name: String,
}

/// Channel override view.
#[derive(Debug, Serialize)]
struct OverrideBody {
    channel_id: Uuid,
    target_kind: String,
    target: String,
    permission: String,
    allowed: bool,
}

impl From<workspaces::ChannelOverride> for OverrideBody {
    fn from(override_row: workspaces::ChannelOverride) -> Self {
        Self {
            channel_id: override_row.channel_id,
            target_kind: override_row.target_kind,
            target: override_row.target,
            permission: override_row.permission,
            allowed: override_row.allowed,
        }
    }
}

/// `POST /v1/channels/{id}/overrides` request.
#[derive(Debug, Deserialize)]
struct OverrideSetBody {
    target_kind: String,
    target: String,
    permission: String,
    allowed: bool,
}

/// Invite view. The code is shown here at creation and listing time only —
/// treat it as a bearer credential.
#[derive(Debug, Serialize)]
struct InviteBody {
    id: Uuid,
    workspace_id: Uuid,
    code: String,
    created_by: Uuid,
    initial_role: String,
    expires_at: Option<DateTime<Utc>>,
    max_uses: Option<i32>,
    uses: i32,
    revoked: bool,
    created_at: DateTime<Utc>,
}

impl From<workspaces::Invite> for InviteBody {
    fn from(invite: workspaces::Invite) -> Self {
        Self {
            id: invite.id,
            workspace_id: invite.workspace_id,
            code: invite.code,
            created_by: invite.created_by,
            initial_role: invite.initial_role,
            expires_at: invite.expires_at,
            max_uses: invite.max_uses,
            uses: invite.uses,
            revoked: invite.revoked,
            created_at: invite.created_at,
        }
    }
}

/// `POST /v1/workspaces/{id}/invites` request.
#[derive(Debug, Deserialize)]
struct CreateInviteBody {
    initial_role: Option<String>,
    expires_in_secs: Option<i64>,
    max_uses: Option<i32>,
}

/// `POST /v1/workspaces/join` request.
#[derive(Debug, Deserialize)]
struct JoinBody {
    code: String,
}

/// Audit entry view. Actions and ids only, never message plaintext.
#[derive(Debug, Serialize)]
struct AuditBody {
    id: Uuid,
    workspace_id: Uuid,
    actor_id: Uuid,
    action: String,
    target_id: Option<Uuid>,
    detail: serde_json::Value,
    created_at: DateTime<Utc>,
}

impl From<workspaces::AuditEntry> for AuditBody {
    fn from(entry: workspaces::AuditEntry) -> Self {
        Self {
            id: entry.id,
            workspace_id: entry.workspace_id,
            actor_id: entry.actor_id,
            action: entry.action,
            target_id: entry.target_id,
            detail: entry.detail,
            created_at: entry.created_at,
        }
    }
}

/// `GET /v1/workspaces/{id}/audit` query.
#[derive(Debug, Deserialize)]
struct AuditParams {
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
    // Channel conversations are workspace-gated BEFORE transport decoding, so
    // strangers get the membership wall (403) rather than a decode error
    // (400): membership first, then the race-heal, then the SEND write gate.
    if let Some(channel) = workspaces::channel_by_conversation(pool, body.conversation_id).await? {
        let (_workspace, _role) =
            workspaces::get_workspace(pool, bearer.user_id(), channel.workspace_id).await?;
        workspaces::ensure_channel_participation(pool, channel.workspace_id, bearer.user_id())
            .await?;
        if !workspaces::can_send(pool, channel.id, bearer.user_id()).await? {
            return Err(AppError::Denied("not permitted in this channel".to_owned()));
        }
    }
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
    // Channel read gate: workspace members (guests included) may read. The
    // SEND grant is a *write* gate and does not apply to history — guests
    // are read-only, not blind.
    if let Some(channel) = workspaces::channel_by_conversation(pool, params.conversation_id).await?
    {
        let (_workspace, _role) =
            workspaces::get_workspace(pool, bearer.user_id(), channel.workspace_id).await?;
        workspaces::ensure_channel_participation(pool, channel.workspace_id, bearer.user_id())
            .await?;
    }
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

fn workspace_body(workspace: workspaces::Workspace, role: workspaces::Role) -> WorkspaceBody {
    WorkspaceBody {
        id: workspace.id,
        name: workspace.name,
        owner_id: workspace.owner_id,
        my_role: role.as_str().to_owned(),
        created_at: workspace.created_at,
        updated_at: workspace.updated_at,
    }
}

async fn create_workspace(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<WorkspaceNameBody>,
) -> Result<(StatusCode, Json<WorkspaceBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let workspace = workspaces::create_workspace(pool, bearer.user_id(), &body.name).await?;
    Ok((
        StatusCode::CREATED,
        Json(workspace_body(workspace, workspaces::Role::Owner)),
    ))
}

async fn list_workspaces(
    State(state): State<AppState>,
    bearer: Bearer,
) -> Result<Json<Vec<WorkspaceBody>>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let rows = workspaces::list_workspaces(pool, bearer.user_id()).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(workspace, role)| workspace_body(workspace, role))
            .collect(),
    ))
}

async fn get_workspace(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<WorkspaceBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let (workspace, role) = workspaces::get_workspace(pool, bearer.user_id(), workspace_id).await?;
    Ok(Json(workspace_body(workspace, role)))
}

async fn rename_workspace(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<WorkspaceNameBody>,
) -> Result<Json<WorkspaceBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let workspace =
        workspaces::rename_workspace(pool, bearer.user_id(), workspace_id, &body.name).await?;
    let role = workspaces::role_of(pool, workspace_id, bearer.user_id())
        .await?
        .ok_or(AppError::Forbidden)?;
    Ok(Json(workspace_body(workspace, role)))
}

async fn add_workspace_member(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<AddMemberBody>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let role = workspaces::parse_role(&body.role)?;
    // Membership gate BEFORE handle resolution: resolving first would let any
    // authenticated stranger distinguish registered handles (400) from
    // unregistered ones only after the fact — resolve only for managers.
    workspaces::require(
        pool,
        workspace_id,
        bearer.user_id(),
        workspaces::Permission::ManageMembers,
    )
    .await?;
    let target = auth::user_id_by_handle(pool, &body.user_handle).await?;
    workspaces::add_member(pool, bearer.user_id(), workspace_id, target, role).await?;
    Ok(StatusCode::CREATED)
}

async fn set_workspace_role(
    State(state): State<AppState>,
    bearer: Bearer,
    Path((workspace_id, user_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<SetRoleBody>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let role = workspaces::parse_role(&body.role)?;
    workspaces::set_role(pool, bearer.user_id(), workspace_id, user_id, role).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn kick_workspace_member(
    State(state): State<AppState>,
    bearer: Bearer,
    Path((workspace_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    workspaces::remove_member(pool, bearer.user_id(), workspace_id, user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave_workspace(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    workspaces::leave(pool, bearer.user_id(), workspace_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ban_workspace_member(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<BanBody>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    workspaces::ban_member(
        pool,
        bearer.user_id(),
        workspace_id,
        body.user_id,
        body.reason.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(StatusCode::CREATED)
}

async fn unban_workspace_member(
    State(state): State<AppState>,
    bearer: Bearer,
    Path((workspace_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    workspaces::unban(pool, bearer.user_id(), workspace_id, user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_channel(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<ChannelNameBody>,
) -> Result<(StatusCode, Json<ChannelBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let channel =
        workspaces::create_channel(pool, bearer.user_id(), workspace_id, &body.name).await?;
    Ok((StatusCode::CREATED, Json(ChannelBody::from(channel))))
}

async fn list_channels(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<ChannelBody>>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let channels = workspaces::list_channels(pool, bearer.user_id(), workspace_id).await?;
    Ok(Json(channels.into_iter().map(ChannelBody::from).collect()))
}

async fn get_channel(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(channel_id): Path<Uuid>,
) -> Result<Json<ChannelBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let channel = workspaces::get_channel(pool, bearer.user_id(), channel_id).await?;
    Ok(Json(ChannelBody::from(channel)))
}

async fn rename_channel(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<ChannelNameBody>,
) -> Result<Json<ChannelBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let channel =
        workspaces::rename_channel(pool, bearer.user_id(), channel_id, &body.name).await?;
    Ok(Json(ChannelBody::from(channel)))
}

async fn delete_channel(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(channel_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    workspaces::delete_channel(pool, bearer.user_id(), channel_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_channel_override(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<OverrideSetBody>,
) -> Result<(StatusCode, Json<OverrideBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let permission = workspaces::parse_permission(&body.permission)?;
    let override_row = workspaces::set_override(
        pool,
        bearer.user_id(),
        channel_id,
        &body.target_kind,
        &body.target,
        permission,
        body.allowed,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(OverrideBody::from(override_row))))
}

async fn list_channel_overrides(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(channel_id): Path<Uuid>,
) -> Result<Json<Vec<OverrideBody>>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let rows = workspaces::list_overrides(pool, bearer.user_id(), channel_id).await?;
    Ok(Json(rows.into_iter().map(OverrideBody::from).collect()))
}

async fn delete_channel_override(
    State(state): State<AppState>,
    bearer: Bearer,
    Path((channel_id, target_kind, target, permission)): Path<(Uuid, String, String, String)>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let permission = workspaces::parse_permission(&permission)?;
    workspaces::delete_override(
        pool,
        bearer.user_id(),
        channel_id,
        &target_kind,
        &target,
        permission,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_invite(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<CreateInviteBody>,
) -> Result<(StatusCode, Json<InviteBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let initial = body.initial_role.as_deref().unwrap_or("member");
    let role = workspaces::parse_role(initial)?;
    // The invite code is returned once here (and on manager listing); the
    // request log must never carry it — only the response does.
    let invite = workspaces::create_invite(
        pool,
        bearer.user_id(),
        workspace_id,
        role,
        body.expires_in_secs,
        body.max_uses,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(InviteBody::from(invite))))
}

async fn list_invites(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<InviteBody>>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let invites = workspaces::list_invites(pool, bearer.user_id(), workspace_id).await?;
    Ok(Json(invites.into_iter().map(InviteBody::from).collect()))
}

async fn revoke_invite(
    State(state): State<AppState>,
    bearer: Bearer,
    Path((workspace_id, invite_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    workspaces::revoke_invite(pool, bearer.user_id(), workspace_id, invite_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn join_workspace(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<JoinBody>,
) -> Result<(StatusCode, Json<WorkspaceBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // The invite code arrives in the request body (never a URL) so it stays
    // out of access logs; it is never logged or echoed back here.
    let (workspace, role) = workspaces::join_via_invite(pool, bearer.user_id(), &body.code).await?;
    Ok((StatusCode::OK, Json(workspace_body(workspace, role))))
}

async fn list_audit(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Query(params): Query<AuditParams>,
) -> Result<Json<Vec<AuditBody>>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let entries = workspaces::list_audit(
        pool,
        bearer.user_id(),
        workspace_id,
        params.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(entries.into_iter().map(AuditBody::from).collect()))
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
        .route(
            "/v1/workspaces",
            post(create_workspace).get(list_workspaces),
        )
        .route("/v1/workspaces/join", post(join_workspace))
        .route(
            "/v1/workspaces/{id}",
            get(get_workspace).patch(rename_workspace),
        )
        .route("/v1/workspaces/{id}/members", post(add_workspace_member))
        .route("/v1/workspaces/{id}/leave", post(leave_workspace))
        .route(
            "/v1/workspaces/{id}/members/{user_id}",
            patch(set_workspace_role).delete(kick_workspace_member),
        )
        .route("/v1/workspaces/{id}/bans", post(ban_workspace_member))
        .route(
            "/v1/workspaces/{id}/bans/{user_id}",
            delete(unban_workspace_member),
        )
        .route(
            "/v1/workspaces/{id}/channels",
            post(create_channel).get(list_channels),
        )
        .route(
            "/v1/channels/{id}",
            get(get_channel)
                .patch(rename_channel)
                .delete(delete_channel),
        )
        .route(
            "/v1/channels/{id}/overrides",
            post(set_channel_override).get(list_channel_overrides),
        )
        .route(
            "/v1/channels/{id}/overrides/{target_kind}/{target}/{permission}",
            delete(delete_channel_override),
        )
        .route(
            "/v1/workspaces/{id}/invites",
            post(create_invite).get(list_invites),
        )
        .route(
            "/v1/workspaces/{id}/invites/{invite_id}",
            delete(revoke_invite),
        )
        .route("/v1/workspaces/{id}/audit", get(list_audit))
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

        // Device registration behind the bearer → 201; bad pubkey → 400.
        // Both keys travel as 64-char hex (Ed25519 identity + X25519 agreement).
        let agree_hex = "cd".repeat(32);
        let (status, device) = post_json(
            build_router(state.clone()),
            "/v1/auth/devices",
            serde_json::json!({"label": "laptop", "identity_pubkey": "ab".repeat(32), "agreement_pubkey": agree_hex}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(device["label"], "laptop");
        assert_eq!(device["agreement_pubkey"], "cd".repeat(32));
        let (status, _) = post_json(
            build_router(state.clone()),
            "/v1/auth/devices",
            serde_json::json!({"label": "bad", "identity_pubkey": "zz", "agreement_pubkey": agree_hex}),
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

    /// The outbox table is global to the test database and each realtime test
    /// spawns a worker that claims rows for its own hub: running two such
    /// tests concurrently lets the workers steal each other's live events.
    /// This guard serializes them; replay-only tests are unaffected.
    static WS_SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

    /// Hold while a realtime test owns the shared outbox worker pattern.
    async fn ws_guard() -> tokio::sync::MutexGuard<'static, ()> {
        WS_SERIAL
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
    }

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
        let _serial = ws_guard().await;
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

    async fn authed_json(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        token: &str,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let body = body.map_or_else(axum::body::Body::empty, |json| {
            axum::body::Body::from(json.to_string())
        });
        let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    /// Same as `authed_json` but for endpoints with empty bodies (`204 No
    /// Content`, or `201` without JSON): returns only the status.
    async fn authed_status(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        token: &str,
    ) -> StatusCode {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let body = body.map_or_else(axum::body::Body::empty, |json| {
            axum::body::Body::from(json.to_string())
        });
        app.oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
            .status()
    }

    /// Register three HTTP users; returns `(founder, member, stranger)` tokens.
    /// `prefix` keeps parallel tests on distinct handles.
    async fn http_users(
        pool: &sqlx::PgPool,
        stamp: &str,
        prefix: &str,
    ) -> (String, String, String) {
        for (tag, pw) in [
            ("phil", "pw-http-1"),
            ("bob", "pw-http-2"),
            ("mall", "pw-http-3"),
        ] {
            auth::create_user(
                pool,
                &format!("{prefix}{tag}{stamp}"),
                &format!("{prefix}{tag}{stamp}@example.com"),
                tag,
                pw,
            )
            .await
            .expect("register user");
        }
        let login = |handle: String, pw: &'static str| {
            let pool = pool.clone();
            async move { auth::login(&pool, &handle, pw).await.expect("login").token }
        };
        // Sequential awaits: no lifetime escapes the closure call.
        let phil = login(format!("{prefix}phil{stamp}"), "pw-http-1").await;
        let bob = login(format!("{prefix}bob{stamp}"), "pw-http-2").await;
        let mallory = login(format!("{prefix}mall{stamp}"), "pw-http-3").await;
        (phil, bob, mallory)
    }

    /// Create a workspace plus its `#general` channel through the routes.
    /// Returns `(workspace_id, conversation_id)`.
    async fn http_space(state: &AppState, token: &str) -> (String, String) {
        let (status, workspace) = post_json(
            build_router(state.clone()),
            "/v1/workspaces",
            serde_json::json!({ "name": "HTTP Space" }),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(workspace["my_role"], "owner");
        let workspace_id = workspace["id"].as_str().expect("workspace id").to_owned();
        let (status, channel) = post_json(
            build_router(state.clone()),
            &format!("/v1/workspaces/{workspace_id}/channels"),
            serde_json::json!({ "name": "general" }),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let conversation_id = channel["conversation_id"]
            .as_str()
            .expect("convo id")
            .to_owned();
        (workspace_id, conversation_id)
    }

    /// Requires a live database; skips honestly without one. Milestone E happy
    /// path over real routes: workspace → channel → invite → join → channel
    /// send → history.
    #[tokio::test]
    async fn workspace_http_channel_send() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_http_channel_send (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "ha").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        // Single-use invite admits Bob exactly once.
        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({ "max_uses": 1 }),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("invite code").to_owned();

        let (status, joined) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(joined["my_role"], "member");

        // Bob sends through the channel's backing conversation; both read it.
        let (status, sent) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"channel-bro"),
            }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(sent["seq"], 1);

        let (status, history) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(history.as_array().expect("history").len(), 1);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The hostile side
    /// over real routes: strangers hit the same wall everywhere, spent codes
    /// stay silent, and bans go dark immediately.
    #[tokio::test]
    async fn workspace_http_outsider_ban_audit() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_http_outsider_ban_audit (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, mallory) = http_users(&pool, &stamp, "hb").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        // Open invite admits Bob; Mallory stays a stranger.
        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("invite code").to_owned();
        let (status, _) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{workspace_id}"),
            None,
            &mallory,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"mallory-hi"),
            }),
            Some(&mallory),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Founder bans Bob: his history goes dark, his sends die.
        let bob_id = auth::user_id_by_handle(&pool, &format!("hbbob{stamp}"))
            .await
            .expect("bob id");
        let status = authed_status(
            build_router(state()),
            "POST",
            &format!("/v1/workspaces/{workspace_id}/bans"),
            Some(serde_json::json!({ "user_id": bob_id, "reason": "spam" })),
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Audit is founder-visible and carries no envelope bytes.
        let (status, audit) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{workspace_id}/audit?limit=50"),
            None,
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let entries = audit.as_array().expect("audit entries");
        assert!(entries.len() >= 5, "audit has {} entries", entries.len());
        let actions: Vec<&str> = entries
            .iter()
            .filter_map(|entry| entry["action"].as_str())
            .collect();
        assert!(actions.contains(&"member.banned"));
        assert!(actions.contains(&"invite.accepted"));
        for entry in entries {
            assert!(!entry.to_string().contains("ciphertext"));
        }
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. No oracles, no
    /// precedence leaks: strangers get the same workspace-shaped 403 for
    /// unknown handles, unknown workspaces, and undecodable sends.
    #[tokio::test]
    async fn workspace_member_oracle() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_member_oracle (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "hd").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        // Unknown handle as a stranger: 403, not the 400 members get.
        let (status, body) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/members"),
            serde_json::json!({"user_handle": "nobody-here", "role": "member"}),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["message"], "not a workspace member");

        // Unknown workspace id: identical wall, no existence signal.
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{}", Uuid::now_v7()),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Kicked ex-member knows the conversation id: garbage bytes still
        // meet the membership wall first (403), not a decode error (400).
        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("code").to_owned();
        let (status, _) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let bob_id = auth::user_id_by_handle(&pool, &format!("hdbob{stamp}"))
            .await
            .expect("bob id");
        let status = authed_status(
            build_router(state()),
            "POST",
            &format!("/v1/workspaces/{workspace_id}/bans"),
            Some(serde_json::json!({ "user_id": bob_id })),
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": "!!!not-base64!!!",
            }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Guests are
    /// read-only, not blind: history reads succeed, sends are refused, and a
    /// stranded seat heals on the next read instead of locking them out.
    #[tokio::test]
    async fn workspace_guest_reads_but_not_writes() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_guest_reads_but_not_writes (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "he").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({ "initial_role": "guest" }),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("code").to_owned();
        let (status, joined) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(joined["my_role"], "guest");

        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"owner-says-hi"),
            }),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        // Guest reads fine…
        let (status, history) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(history.as_array().expect("history").len(), 1);
        // …but cannot write.
        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"guest-tries"),
            }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Strand the guest's seat; the next read heals and succeeds.
        let bob_id = auth::user_id_by_handle(&pool, &format!("hebob{stamp}"))
            .await
            .expect("bob id");
        sqlx::query(
            "DELETE FROM conversation_participants
             WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(conversation_id.parse::<Uuid>().expect("convo uuid"))
        .bind(bob_id)
        .execute(&pool)
        .await
        .expect("strand guest");
        let (status, history) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(history.as_array().expect("history").len(), 1);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The leave path:
    /// a member exits voluntarily (204, then 403 everywhere); the last owner
    /// is stopped with a 400.
    #[tokio::test]
    async fn workspace_leave_route() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_leave_route (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "hf").await;
        let (workspace_id, _) = http_space(&state(), &phil).await;

        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("code").to_owned();
        let (status, _) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let status = authed_status(
            build_router(state()),
            "POST",
            &format!("/v1/workspaces/{workspace_id}/leave"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{workspace_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/leave"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        pool.close().await;
    }

    /// Register one gateway user and return `(token, id)`.
    async fn gw_login(pool: &sqlx::PgPool, tag: &str, stamp: &str) -> (String, Uuid) {
        let user = auth::create_user(
            pool,
            &format!("{tag}{stamp}"),
            &format!("{tag}{stamp}@example.com"),
            tag,
            "pw-gw-1",
        )
        .await
        .expect("register user");
        let token = auth::login(pool, &format!("{tag}{stamp}"), "pw-gw-1")
            .await
            .expect("login")
            .token;
        (token, user.id)
    }

    /// Requires a live database; skips honestly without one. Gateway parity
    /// with HTTP: a guest replays channel events it may read, and a banned
    /// member's open connection goes quiet — kicks take effect live, without
    /// a reconnect.
    #[tokio::test]
    async fn gateway_channel_guest_and_ban() {
        let _serial = ws_guard().await;
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: gateway_channel_guest_and_ban (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil_token, phil_id) = gw_login(&pool, "gwphil", &stamp).await;
        let (guest_token, guest_id) = gw_login(&pool, "gwguest", &stamp).await;
        let (victim_token, victim_id) = gw_login(&pool, "gwvic", &stamp).await;

        let workspace = workspaces::create_workspace(&pool, phil_id, "GW Space")
            .await
            .expect("create workspace");
        workspaces::add_member(
            &pool,
            phil_id,
            workspace.id,
            guest_id,
            workspaces::Role::Guest,
        )
        .await
        .expect("add guest");
        workspaces::add_member(
            &pool,
            phil_id,
            workspace.id,
            victim_id,
            workspaces::Role::Member,
        )
        .await
        .expect("add victim");
        let channel = workspaces::create_channel(&pool, phil_id, workspace.id, "general")
            .await
            .expect("create channel");

        let (hub, _) = broadcast::channel(HUB_CAPACITY);
        let state = AppState {
            pool: Some(pool.clone()),
            hub: hub.clone(),
        };
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

        http_send(
            &http_state,
            &phil_token,
            channel.conversation_id,
            &STANDARD.encode(b"one"),
        )
        .await;
        // Guest replays what it may read; victim replays as a member.
        let mut guest_ws = connect_identified(&addr, &guest_token, None).await;
        assert_eq!(
            ws_next(&mut guest_ws).await["event"]["payload"]["data"]["seq"],
            1
        );
        let mut victim_ws = connect_identified(&addr, &victim_token, None).await;
        assert_eq!(
            ws_next(&mut victim_ws).await["event"]["payload"]["data"]["seq"],
            1
        );

        // Ban lands live: the victim's open connection goes quiet while the
        // guest's keeps streaming.
        workspaces::ban_member(&pool, phil_id, workspace.id, victim_id, "spam")
            .await
            .expect("ban victim");
        http_send(
            &http_state,
            &phil_token,
            channel.conversation_id,
            &STANDARD.encode(b"two"),
        )
        .await;
        expect_event(&mut guest_ws, 2).await;
        let quiet =
            tokio::time::timeout(std::time::Duration::from_secs(3), ws_next(&mut victim_ws)).await;
        assert!(quiet.is_err(), "banned connection must go quiet");

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
