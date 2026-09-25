//! Messaging HTTP adapters: DMs, groups, send, and history.
//!
//! Thin adapters over `crate::messaging` (with workspace send/read gates from
//! `crate::workspaces`). No behavior changes from the former `routes.rs`.

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::collections::HashSet;

use crate::auth;
use crate::errors::AppError;
use crate::messaging;
use crate::state::{AppState, Bearer};
use crate::workspaces;

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
    /// The caller's own read marker; never another member's.
    last_read_seq: i64,
    /// `dm`/`group` rosters so clients can name senders; empty for channels.
    member_profiles: Vec<MemberProfileBody>,
}

#[derive(Debug, Serialize)]
struct MemberProfileBody {
    user_id: Uuid,
    handle: String,
    display_name: String,
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
            last_read_seq: row.last_read_seq,
            member_profiles: row
                .member_profiles
                .into_iter()
                .map(|profile| MemberProfileBody {
                    user_id: profile.user_id,
                    handle: profile.handle,
                    display_name: profile.display_name,
                })
                .collect(),
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

/// Other members a single group-creation request may name. Matches the
/// client's `MAX_GROUP_MEMBERS`; a group DM is small, not a workspace.
const MAX_GROUP_HANDLES: usize = 50;

async fn create_group(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<GroupBody>,
) -> Result<(StatusCode, Json<ConversationBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Bound the work (one lookup per handle) before doing any of it.
    if body.member_handles.len() > MAX_GROUP_HANDLES {
        return Err(AppError::BadRequest(format!(
            "a group can start with at most {MAX_GROUP_HANDLES} other members"
        )));
    }
    // Handles are case-insensitive, so `Ana`/`ana`/`@CREATOR` spellings can
    // resolve to the same account: dedupe by id and never count the creator.
    let mut seen = HashSet::with_capacity(body.member_handles.len());
    let mut members = Vec::with_capacity(body.member_handles.len());
    for handle in &body.member_handles {
        let id = auth::user_id_by_handle(pool, handle).await?;
        if id != bearer.user_id() && seen.insert(id) {
            members.push(id);
        }
    }
    // A group DM has 3+ participants; two people talk in a DM.
    if members.len() < 2 {
        return Err(AppError::BadRequest(
            "a group needs at least two other members; use a DM for one".to_owned(),
        ));
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

/// `POST /v1/conversations/{id}/read` request: the highest seq now read.
#[derive(Debug, Deserialize)]
struct ReadBody {
    seq: i64,
}

/// The caller's stored (monotonic, clamped) read marker.
#[derive(Debug, Serialize)]
struct ReadMarkerBody {
    conversation_id: Uuid,
    last_read_seq: i64,
}

async fn mark_read(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(conversation_id): Path<Uuid>,
    Json(body): Json<ReadBody>,
) -> Result<Json<ReadMarkerBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Same read gate as history: workspace members may mark channels read,
    // strangers get the membership wall rather than a participant oracle.
    if let Some(channel) = workspaces::channel_by_conversation(pool, conversation_id).await? {
        let (_workspace, _role) =
            workspaces::get_workspace(pool, bearer.user_id(), channel.workspace_id).await?;
        workspaces::ensure_channel_participation(pool, channel.workspace_id, bearer.user_id())
            .await?;
    }
    let last_read_seq =
        messaging::mark_read(pool, bearer.user_id(), conversation_id, body.seq).await?;
    Ok(Json(ReadMarkerBody {
        conversation_id,
        last_read_seq,
    }))
}

/// Relay "I am typing" to the conversation's other live members. Gated
/// exactly like sending: a channel needs workspace membership and SEND, any
/// other conversation needs participation. Ephemeral: nothing is stored.
async fn typing(
    State(state): State<AppState>,
    Extension(bus): Extension<crate::typing::TypingBus>,
    bearer: Bearer,
    Path(conversation_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    if let Some(channel) = workspaces::channel_by_conversation(pool, conversation_id).await? {
        let (_workspace, _role) =
            workspaces::get_workspace(pool, bearer.user_id(), channel.workspace_id).await?;
        workspaces::ensure_channel_participation(pool, channel.workspace_id, bearer.user_id())
            .await?;
        if !workspaces::can_send(pool, channel.id, bearer.user_id()).await? {
            return Err(AppError::Denied("not permitted in this channel".to_owned()));
        }
    } else if !messaging::is_member(pool, conversation_id, bearer.user_id()).await? {
        return Err(messaging::MessagingError::NotMember.into());
    }
    let recipients = messaging::participant_ids(pool, conversation_id).await?;
    bus.publish(crate::typing::TypingSignal {
        conversation_id,
        user_id: bearer.user_id(),
        recipients: recipients.into(),
    });
    Ok(StatusCode::NO_CONTENT)
}

/// Messaging routes: conversations plus send/history and read markers.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/conversations/dm", post(create_dm))
        .route(
            "/v1/conversations",
            post(create_group).get(list_conversations),
        )
        .route("/v1/conversations/{id}/read", post(mark_read))
        .route("/v1/conversations/{id}/typing", post(typing))
        .route("/v1/messages", post(send_message).get(message_history))
}
