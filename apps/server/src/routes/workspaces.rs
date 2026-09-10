//! Workspace HTTP adapters: spaces, members, channels, invites, audit.
//!
//! Thin adapters over `crate::workspaces` (handle resolution via
//! `crate::auth`). No behavior changes from the former `routes.rs`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth;
use crate::errors::AppError;
use crate::state::{AppState, Bearer};
use crate::workspaces;

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

/// Workspace/channel/invite/audit routes.
#[allow(clippy::too_many_lines)]
pub fn router() -> Router<AppState> {
    Router::new()
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
}
