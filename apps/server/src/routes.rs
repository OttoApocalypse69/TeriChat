//! HTTP routes: router construction plus every handler and its
//! request/response shapes.
//!
//! This is the HTTP adapter layer: handlers authenticate via [`Bearer`](crate::state::Bearer),
//! fail via [`AppError`](crate::errors::AppError), and delegate all domain
//! logic to the domain modules. Probes live in [`crate::health`].

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth;
use crate::errors::AppError;
use crate::gateway;
use crate::health::{health, ready};
use crate::messaging;
use crate::state::{AppState, Bearer};
use crate::stats;
use crate::workspaces;

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

/// `GET /v1/conversations` entry: the caller's conversation with, for
/// two-member DMs, the peer handle + display name resolved server-side
/// (caller is a member — no handle oracle), plus the last message position
/// for previews and sync.
#[derive(Debug, Serialize)]
struct ConversationSummaryBody {
    id: Uuid,
    kind: String,
    members: Vec<Uuid>,
    peer_handle: Option<String>,
    peer_display_name: Option<String>,
    last_seq: Option<i64>,
    last_sent_at: Option<DateTime<Utc>>,
}

impl From<messaging::ConversationSummary> for ConversationSummaryBody {
    fn from(row: messaging::ConversationSummary) -> Self {
        Self {
            id: row.id,
            kind: row.kind,
            members: row.members,
            peer_handle: row.peer_handle,
            peer_display_name: row.peer_display_name,
            last_seq: row.last_seq,
            last_sent_at: row.last_sent_at,
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

/// Public roster profile; intentionally separate from the account response.
#[derive(Debug, Serialize)]
struct WorkspaceMemberBody {
    user_id: Uuid,
    handle: String,
    display_name: String,
    role: String,
    joined_at: DateTime<Utc>,
}

impl From<workspaces::Member> for WorkspaceMemberBody {
    fn from(member: workspaces::Member) -> Self {
        Self {
            user_id: member.user_id,
            handle: member.handle,
            display_name: member.display_name,
            role: member.role,
            joined_at: member.joined_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct WorkspaceMembersBody {
    members: Vec<WorkspaceMemberBody>,
    next_cursor: Option<Uuid>,
}

/// `GET /v1/workspaces/{id}/members`: ascending UUID keyset pagination.
#[derive(Debug, Deserialize)]
struct WorkspaceMembersParams {
    after: Option<Uuid>,
    limit: Option<i64>,
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
        serde_json::json!({ "status": "ok", "session_id": bearer.session_id() }),
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

async fn list_conversations(
    State(state): State<AppState>,
    bearer: Bearer,
) -> Result<Json<Vec<ConversationSummaryBody>>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Caller-scoped by construction: only conversations the caller belongs
    // to are returned, with peer identity resolved server-side (no oracle).
    let rows = messaging::list_conversations(pool, bearer.user_id()).await?;
    Ok(Json(
        rows.into_iter()
            .map(ConversationSummaryBody::from)
            .collect(),
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

async fn list_workspace_members(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Query(params): Query<WorkspaceMembersParams>,
) -> Result<Json<WorkspaceMembersBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let page = workspaces::list_members(
        pool,
        bearer.user_id(),
        workspace_id,
        params.after,
        params.limit.unwrap_or(100),
    )
    .await?;
    Ok(Json(WorkspaceMembersBody {
        members: page
            .members
            .into_iter()
            .map(WorkspaceMemberBody::from)
            .collect(),
        next_cursor: page.next_cursor,
    }))
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

async fn get_own_stats(
    State(state): State<AppState>,
    bearer: Bearer,
) -> Result<Json<StatsBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Scoped to the caller by construction: only their own id is queried.
    let rollup = stats::own_stats(pool, bearer.user_id()).await?;
    Ok(Json(StatsBody {
        user_id: rollup.user_id,
        message_count: rollup.message_count,
        last_message_at: rollup.last_message_at,
    }))
}

async fn get_conversation_stats(
    State(state): State<AppState>,
    bearer: Bearer,
    Query(params): Query<ConversationStatsParams>,
) -> Result<Json<ConversationStatsBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Membership-checked inside; outsiders get `Forbidden`, never a count.
    let scoped = stats::conversation_stats(pool, bearer.user_id(), params.conversation_id).await?;
    Ok(Json(ConversationStatsBody {
        user_id: scoped.user_id,
        conversation_id: scoped.conversation_id,
        message_count: scoped.message_count,
        last_message_at: scoped.last_message_at,
    }))
}

/// `GET /v1/stats/me` view: the caller's own message rollup. Counts only,
/// never content — and only the caller's own.
#[derive(Debug, Serialize)]
struct StatsBody {
    user_id: Uuid,
    message_count: i64,
    last_message_at: Option<DateTime<Utc>>,
}

/// `GET /v1/stats/conversation` view: the caller's own sends in one conversation.
#[derive(Debug, Serialize)]
struct ConversationStatsBody {
    user_id: Uuid,
    conversation_id: Uuid,
    message_count: i64,
    last_message_at: Option<DateTime<Utc>>,
}

/// `GET /v1/stats/conversation` query.
#[derive(Debug, Deserialize)]
struct ConversationStatsParams {
    conversation_id: Uuid,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/auth/devices", post(register_device))
        .route("/v1/conversations/dm", post(create_dm))
        .route(
            "/v1/conversations",
            post(create_group).get(list_conversations),
        )
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
        .route(
            "/v1/workspaces/{id}/members",
            post(add_workspace_member).get(list_workspace_members),
        )
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
        .route("/v1/stats/me", get(get_own_stats))
        .route("/v1/stats/conversation", get(get_conversation_stats))
        .with_state(state)
}

#[cfg(test)]
mod member_tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    struct Fixture {
        pool: sqlx::PgPool,
        app: Router,
        workspace: Uuid,
        owner: Uuid,
        owner_token: String,
        guest: Uuid,
        guest_handle: String,
        guest_token: String,
    }

    async fn fixture() -> Option<Fixture> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("SKIPPED: workspace member route tests (DATABASE_URL unset)");
            return None;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("member route database");
        crate::MIGRATOR.run(&pool).await.expect("migrations");
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut users = Vec::new();
        for name in ["mhttpowner", "mhttpguest"] {
            let handle = format!("{name}{stamp}");
            let user = auth::create_user(
                &pool,
                &handle,
                &format!("{handle}@example.invalid"),
                name,
                "synthetic-route-password",
            )
            .await
            .expect("synthetic account");
            let token = auth::login(&pool, &handle, "synthetic-route-password")
                .await
                .expect("login")
                .token;
            users.push((user, token));
        }
        let (guest, guest_token) = users.pop().expect("guest");
        let (owner, owner_token) = users.pop().expect("owner");
        let workspace = workspaces::create_workspace(&pool, owner.id, "HTTP roster")
            .await
            .expect("workspace")
            .id;
        let (hub, _) = tokio::sync::broadcast::channel(crate::HUB_CAPACITY);
        let app = build_router(AppState {
            pool: Some(pool.clone()),
            hub,
        });
        Some(Fixture {
            pool,
            app,
            workspace,
            owner: owner.id,
            owner_token,
            guest: guest.id,
            guest_handle: guest.handle,
            guest_token,
        })
    }

    async fn request(
        f: &Fixture,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, Vec<u8>) {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let body = body.map_or_else(Body::empty, |json| Body::from(json.to_string()));
        let response = f
            .app
            .clone()
            .oneshot(
                request
                    .header("content-type", "application/json")
                    .body(body)
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        (
            status,
            response
                .into_body()
                .collect()
                .await
                .expect("response bytes")
                .to_bytes()
                .to_vec(),
        )
    }

    #[tokio::test]
    async fn members_route_pagination_public_fields_and_post_preserved() {
        let Some(f) = fixture().await else { return };
        let uri = format!("/v1/workspaces/{}/members", f.workspace);
        let (status, _) = request(
            &f,
            "POST",
            &uri,
            Some(&f.owner_token),
            Some(serde_json::json!({"user_handle": f.guest_handle, "role": "guest"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, bytes) = request(
            &f,
            "GET",
            &format!("{uri}?limit=1"),
            Some(&f.guest_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let first: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(first.as_object().expect("page object").len(), 2);
        assert_eq!(first["members"].as_array().expect("members").len(), 1);
        let first_id = first["members"][0]["user_id"].as_str().expect("first id");
        assert_eq!(first["next_cursor"], first_id);
        let (status, bytes) = request(
            &f,
            "GET",
            &format!("{uri}?after={first_id}&limit=1"),
            Some(&f.guest_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let last: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert!(last["next_cursor"].is_null());
        assert_eq!(last["members"].as_array().expect("members").len(), 1);
        assert!(first_id < last["members"][0]["user_id"].as_str().expect("last id"));
        for page in [&first, &last] {
            let profile = page["members"][0].as_object().expect("profile");
            let mut keys: Vec<&str> = profile.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                ["display_name", "handle", "joined_at", "role", "user_id"]
            );
            DateTime::parse_from_rfc3339(profile["joined_at"].as_str().expect("join time"))
                .expect("valid timestamp");
        }
        let (status, bytes) = request(&f, "GET", &uri, Some(&f.owner_token), None).await;
        assert_eq!(status, StatusCode::OK);
        let all: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(all["members"].as_array().expect("members").len(), 2);
        assert!(all["next_cursor"].is_null());
    }

    #[tokio::test]
    async fn members_route_rejects_outsiders_revoked_and_bad_queries() {
        let Some(f) = fixture().await else { return };
        let uri = format!("/v1/workspaces/{}/members", f.workspace);
        assert_eq!(
            request(&f, "GET", &uri, None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        let outside = request(&f, "GET", &uri, Some(&f.guest_token), None).await;
        let missing = request(
            &f,
            "GET",
            &format!("/v1/workspaces/{}/members", Uuid::now_v7()),
            Some(&f.guest_token),
            None,
        )
        .await;
        assert_eq!(outside.0, StatusCode::FORBIDDEN);
        assert_eq!(outside, missing, "no workspace existence oracle");
        for query in ["after=not-a-uuid", "limit=0", "limit=-1", "limit=nope"] {
            assert_eq!(
                request(
                    &f,
                    "GET",
                    &format!("{uri}?{query}"),
                    Some(&f.owner_token),
                    None
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        workspaces::add_member(
            &f.pool,
            f.owner,
            f.workspace,
            f.guest,
            workspaces::Role::Guest,
        )
        .await
        .expect("join");
        workspaces::leave(&f.pool, f.guest, f.workspace)
            .await
            .expect("leave");
        assert_eq!(
            request(&f, "GET", &uri, Some(&f.guest_token), None).await,
            outside
        );
        workspaces::add_member(
            &f.pool,
            f.owner,
            f.workspace,
            f.guest,
            workspaces::Role::Guest,
        )
        .await
        .expect("rejoin");
        workspaces::ban_member(&f.pool, f.owner, f.workspace, f.guest, "synthetic ban")
            .await
            .expect("ban");
        let banned = request(&f, "GET", &uri, Some(&f.guest_token), None).await;
        assert_eq!(banned.0, StatusCode::FORBIDDEN);
        let json: serde_json::Value = serde_json::from_slice(&banned.1).expect("error json");
        assert_eq!(json["error"]["message"], "banned from this workspace");
        assert!(json.get("members").is_none());
    }
}
