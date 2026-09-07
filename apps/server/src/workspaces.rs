//! Milestone E workspaces: governed spaces with roles, text channels, invites,
//! channel permission overrides, bans, and a minimal audit log.
//!
//! Trust posture: workspace/channel ids are unguessable (`UUIDv7`) but NOT
//! secret — every mutating check is membership-gated, and non-members get
//! [`WorkspacesError::NotMember`] whether the workspace exists or not (no
//! existence oracle). Banned users are denied before any permission grant.
//! Audit rows carry actions and ids only, never message plaintext.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Workspace roles, weakest first. Rank order is load-bearing: management
/// actions require the actor to strictly outrank both the target's current
/// role and the granted role (the owner bypasses this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Read-only by default; needs an explicit override to send.
    Guest,
    /// Base member: can read and send where not overridden.
    Member,
    /// Can kick members/guests, manage messages, view the audit log.
    Moderator,
    /// Full control below owner: roles, members, bans, channels, invites.
    Admin,
    /// Workspace founder rank. Bypasses the rank rule.
    Owner,
}

impl Role {
    /// Canonical lowercase name, as stored in `workspace_members.role`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Guest => "guest",
            Self::Member => "member",
            Self::Moderator => "moderator",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    /// Hierarchy rank. Higher outranks lower; compare with `rank()`.
    #[must_use]
    pub fn rank(self) -> u8 {
        match self {
            Self::Guest => 0,
            Self::Member => 1,
            Self::Moderator => 2,
            Self::Admin => 3,
            Self::Owner => 4,
        }
    }
}

/// Parse a user-supplied role name (trimmed, case-insensitive).
///
/// # Errors
///
/// Returns [`WorkspacesError::BadInput`] for anything outside the five roles.
pub fn parse_role(raw: &str) -> Result<Role, WorkspacesError> {
    match raw.trim().to_lowercase().as_str() {
        "guest" => Ok(Role::Guest),
        "member" => Ok(Role::Member),
        "moderator" => Ok(Role::Moderator),
        "admin" => Ok(Role::Admin),
        "owner" => Ok(Role::Owner),
        _ => Err(WorkspacesError::BadInput(
            "role must be guest, member, moderator, admin, or owner".to_owned(),
        )),
    }
}

/// Central permission set (subset of `05_WORKSPACES_AND_COLLABORATION.md`
/// used by Alpha; voice/music/economy permissions arrive with those slices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Rename the workspace.
    ManageWorkspace,
    /// Create/rename/delete channels and manage their overrides.
    ManageChannels,
    /// Change member roles.
    ManageRoles,
    /// Add members directly and manage invites.
    ManageMembers,
    /// Remove (kick) members below the actor's rank.
    KickMembers,
    /// Ban/unban members below the actor's rank.
    BanMembers,
    /// Send messages in channels (overridable per channel).
    SendMessages,
    /// Moderate channel messages (overridable per channel).
    ManageMessages,
    /// Read the workspace audit log.
    ViewAuditLog,
}

impl Permission {
    /// Canonical `SCREAMING` name, as stored in `channel_overrides.permission`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManageWorkspace => "MANAGE_WORKSPACE",
            Self::ManageChannels => "MANAGE_CHANNELS",
            Self::ManageRoles => "MANAGE_ROLES",
            Self::ManageMembers => "MANAGE_MEMBERS",
            Self::KickMembers => "KICK_MEMBERS",
            Self::BanMembers => "BAN_MEMBERS",
            Self::SendMessages => "SEND_MESSAGES",
            Self::ManageMessages => "MANAGE_MESSAGES",
            Self::ViewAuditLog => "VIEW_AUDIT_LOG",
        }
    }
}

/// Parse a user-supplied permission name.
///
/// # Errors
///
/// Returns [`WorkspacesError::BadInput`] for unknown permissions.
pub fn parse_permission(raw: &str) -> Result<Permission, WorkspacesError> {
    match raw.trim().to_uppercase().as_str() {
        "MANAGE_WORKSPACE" => Ok(Permission::ManageWorkspace),
        "MANAGE_CHANNELS" => Ok(Permission::ManageChannels),
        "MANAGE_ROLES" => Ok(Permission::ManageRoles),
        "MANAGE_MEMBERS" => Ok(Permission::ManageMembers),
        "KICK_MEMBERS" => Ok(Permission::KickMembers),
        "BAN_MEMBERS" => Ok(Permission::BanMembers),
        "SEND_MESSAGES" => Ok(Permission::SendMessages),
        "MANAGE_MESSAGES" => Ok(Permission::ManageMessages),
        "VIEW_AUDIT_LOG" => Ok(Permission::ViewAuditLog),
        _ => Err(WorkspacesError::BadInput("unknown permission".to_owned())),
    }
}

/// The central permission evaluator: base grant by role. Channel overrides
/// (checked separately in [`channel_allowed`]) can only narrow or widen
/// [`Permission::SendMessages`] and [`Permission::ManageMessages`].
#[must_use]
pub fn role_has(role: Role, permission: Permission) -> bool {
    match role {
        Role::Owner | Role::Admin => true,
        Role::Moderator => matches!(
            permission,
            Permission::SendMessages
                | Permission::ManageMessages
                | Permission::KickMembers
                | Permission::ManageMembers
                | Permission::ViewAuditLog
        ),
        // Plain members send and read; everything else needs a grant.
        Role::Member => matches!(permission, Permission::SendMessages),
        Role::Guest => false,
    }
}

/// Typed workspace error. `NotMember` doubles as "no such workspace" so
/// callers cannot probe for workspace existence.
#[derive(Debug)]
pub enum WorkspacesError {
    /// Caller is not a member (also covers missing workspaces/channels).
    NotMember,
    /// Member lacks the required permission or rank.
    Forbidden,
    /// Caller is banned from this workspace.
    Banned,
    /// Invite code unknown, revoked, expired, or fully used.
    InviteRejected,
    /// Caller-supplied value rejected (names, roles, limits).
    BadInput(String),
    /// Database failure (logged by the HTTP layer, never shown verbatim).
    Database(sqlx::Error),
}

impl std::fmt::Display for WorkspacesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMember => write!(f, "not a workspace member"),
            Self::Forbidden => write!(f, "insufficient workspace permission"),
            Self::Banned => write!(f, "banned from this workspace"),
            Self::InviteRejected => write!(f, "invite is invalid, expired, or fully used"),
            Self::BadInput(detail) => write!(f, "{detail}"),
            Self::Database(_) => write!(f, "database error"),
        }
    }
}

impl std::error::Error for WorkspacesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(err) => Some(err),
            _ => None,
        }
    }
}

/// Workspace row.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Workspace id (`UUIDv7`).
    pub id: Uuid,
    /// Display name (`1..=100` chars).
    pub name: String,
    /// Founding owner account (set at creation, never rewritten: role rows in
    /// `workspace_members` are authoritative for who owns the space today).
    pub owner_id: Uuid,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last rename time.
    pub updated_at: DateTime<Utc>,
}

/// Text channel row with its backing conversation for the message path.
#[derive(Debug, Clone)]
pub struct Channel {
    /// Channel id (`UUIDv7`).
    pub id: Uuid,
    /// Owning workspace.
    pub workspace_id: Uuid,
    /// Backing `conversations` row (`kind = 'channel'`).
    pub conversation_id: Uuid,
    /// Channel name, unique per workspace.
    pub name: String,
    /// Always `'text'` in Alpha.
    pub kind: String,
    /// Creating account.
    pub created_by: Uuid,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// One channel permission override row.
#[derive(Debug, Clone)]
pub struct ChannelOverride {
    /// Owning channel.
    pub channel_id: Uuid,
    /// `'role'` or `'member'`.
    pub target_kind: String,
    /// Role name, or user id in text form.
    pub target: String,
    /// Overridden permission (`SEND_MESSAGES`/`MANAGE_MESSAGES`).
    pub permission: String,
    /// Grant (`true`) or deny (`false`).
    pub allowed: bool,
}

/// Invite row. `code` is the only secret-adjacent value here: random,
/// unguessable, and never logged.
#[derive(Debug, Clone)]
pub struct Invite {
    /// Invite id (`UUIDv7`).
    pub id: Uuid,
    /// Workspace the code admits to.
    pub workspace_id: Uuid,
    /// Random redemption code (URL-safe, shown once at creation).
    pub code: String,
    /// Creating account.
    pub created_by: Uuid,
    /// Role granted on accept.
    pub initial_role: String,
    /// Acceptance deadline, if any.
    pub expires_at: Option<DateTime<Utc>>,
    /// Redemption cap, if any.
    pub max_uses: Option<i32>,
    /// Redemptions so far.
    pub uses: i32,
    /// Revoked codes stop working immediately.
    pub revoked: bool,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Audit entry: who did what to whom, when. Never message plaintext.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// Entry id (`UUIDv7`, time-ordered).
    pub id: Uuid,
    /// Owning workspace.
    pub workspace_id: Uuid,
    /// Acting account.
    pub actor_id: Uuid,
    /// Action tag, e.g. `member.banned`.
    pub action: String,
    /// Affected account, if any.
    pub target_id: Option<Uuid>,
    /// Ids/names/roles only — never secrets or plaintext.
    pub detail: serde_json::Value,
    /// Action time.
    pub created_at: DateTime<Utc>,
}

/// Raw workspace row shared by reads.
type WorkspaceRow = (Uuid, String, Uuid, DateTime<Utc>, DateTime<Utc>);

/// Map a workspace row.
fn to_workspace(row: WorkspaceRow) -> Workspace {
    Workspace {
        id: row.0,
        name: row.1,
        owner_id: row.2,
        created_at: row.3,
        updated_at: row.4,
    }
}

/// Raw channel row shared by reads.
type ChannelRow = (Uuid, Uuid, Uuid, String, String, Uuid, DateTime<Utc>);

/// Map a channel row.
fn to_channel(row: ChannelRow) -> Channel {
    Channel {
        id: row.0,
        workspace_id: row.1,
        conversation_id: row.2,
        name: row.3,
        kind: row.4,
        created_by: row.5,
        created_at: row.6,
    }
}

/// Raw invite row shared by reads.
#[allow(clippy::type_complexity)]
type InviteRow = (
    Uuid,
    Uuid,
    String,
    Uuid,
    String,
    Option<DateTime<Utc>>,
    Option<i32>,
    i32,
    bool,
    DateTime<Utc>,
);

/// Map an invite row.
fn to_invite(row: InviteRow) -> Invite {
    Invite {
        id: row.0,
        workspace_id: row.1,
        code: row.2,
        created_by: row.3,
        initial_role: row.4,
        expires_at: row.5,
        max_uses: row.6,
        uses: row.7,
        revoked: row.8,
        created_at: row.9,
    }
}

/// Raw override row shared by the list query.
type OverrideRow = (String, String, String, bool);

/// Raw audit row shared by the list query.
type AuditRow = (
    Uuid,
    Uuid,
    Uuid,
    String,
    Option<Uuid>,
    serde_json::Value,
    DateTime<Utc>,
);

/// Record an audit entry on any executor (pool or open transaction).
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] when the insert fails.
async fn record_audit<'c, E>(
    ex: E,
    workspace_id: Uuid,
    actor_id: Uuid,
    action: &str,
    target_id: Option<Uuid>,
    detail: &serde_json::Value,
) -> Result<(), WorkspacesError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO workspace_audit (id, workspace_id, actor_id, action, target_id, detail)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(workspace_id)
    .bind(actor_id)
    .bind(action)
    .bind(target_id)
    .bind(detail)
    .execute(ex)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Validate a workspace/channel name: trimmed, `1..=100` chars.
///
/// # Errors
///
/// Returns [`WorkspacesError::BadInput`] when blank or too long.
fn clean_name(raw: &str) -> Result<String, WorkspacesError> {
    let name = raw.trim().to_owned();
    let len = name.chars().count();
    if len == 0 || len > 100 {
        return Err(WorkspacesError::BadInput(
            "name must be 1-100 characters".to_owned(),
        ));
    }
    Ok(name)
}

/// Caller's role in a workspace, or `None` for non-members and missing rows.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
pub async fn role_of(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Option<Role>, WorkspacesError> {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    raw.map(|name| {
        parse_role(&name).map_err(|_| WorkspacesError::Database(sqlx::Error::RowNotFound))
    })
    .transpose()
}

/// Whether `user_id` is banned from `workspace_id`.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
pub async fn is_banned(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<bool, WorkspacesError> {
    let banned: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspace_bans WHERE workspace_id = $1 AND user_id = $2)",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(banned)
}

/// Membership + ban + permission check. Returns the actor's role.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`] for non-members (and missing
/// workspaces), [`WorkspacesError::Banned`] for banned members, or
/// [`WorkspacesError::Forbidden`] when the role lacks `permission`.
pub async fn require(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    permission: Permission,
) -> Result<Role, WorkspacesError> {
    // Ban first: a banned caller is no longer a member, so the role lookup
    // alone would misreport them as a stranger.
    if is_banned(pool, workspace_id, user_id).await? {
        return Err(WorkspacesError::Banned);
    }
    let role = role_of(pool, workspace_id, user_id)
        .await?
        .ok_or(WorkspacesError::NotMember)?;
    if !role_has(role, permission) {
        return Err(WorkspacesError::Forbidden);
    }
    Ok(role)
}

/// Enforce the management rank rule: a non-owner actor must strictly outrank
/// both the target's current role and (for grants) the new role.
///
/// # Errors
///
/// Returns [`WorkspacesError::Forbidden`] when the actor is outranked.
fn check_rank(actor: Role, target: Role, new_role: Option<Role>) -> Result<(), WorkspacesError> {
    if actor == Role::Owner {
        return Ok(());
    }
    if target.rank() >= actor.rank() {
        return Err(WorkspacesError::Forbidden);
    }
    if new_role.is_some_and(|next| next.rank() >= actor.rank()) {
        return Err(WorkspacesError::Forbidden);
    }
    Ok(())
}

/// Count current owners. Guards the last-owner invariant on demote/remove.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
async fn owner_count<'c, E>(ex: E, workspace_id: Uuid) -> Result<i64, WorkspacesError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM workspace_members WHERE workspace_id = $1 AND role = 'owner'",
    )
    .bind(workspace_id)
    .fetch_one(ex)
    .await
    .map_err(WorkspacesError::Database)
}

/// Fresh roles for `actor` and `target` with both rows locked (`FOR UPDATE`,
/// deterministic `user_id` order so concurrent managers cannot deadlock).
/// Management transactions re-check ranks on these values, never on the
/// pre-transaction reads — a promotion racing a kick cannot slip through.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
async fn locked_roles<'c, E>(
    ex: E,
    workspace_id: Uuid,
    actor_id: Uuid,
    target_id: Uuid,
) -> Result<(Option<Role>, Option<Role>), WorkspacesError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT user_id, role FROM workspace_members
         WHERE workspace_id = $1 AND user_id IN ($2, $3)
         ORDER BY user_id FOR UPDATE",
    )
    .bind(workspace_id)
    .bind(actor_id)
    .bind(target_id)
    .fetch_all(ex)
    .await
    .map_err(WorkspacesError::Database)?;
    let mut actor_role = None;
    let mut target_role = None;
    for (user_id, raw) in rows {
        let role =
            parse_role(&raw).map_err(|_| WorkspacesError::Database(sqlx::Error::RowNotFound))?;
        if user_id == actor_id {
            actor_role = Some(role);
        } else {
            target_role = Some(role);
        }
    }
    if actor_id == target_id {
        target_role = actor_role;
    }
    Ok((actor_role, target_role))
}

/// Ensure `user_id` sits in every channel conversation of `workspace_id`.
/// Idempotent self-heal for the join/create-channel race: read and write
/// paths call it before enforcing participation, so a stranded member is
/// re-seated instead of locked out.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
pub async fn ensure_channel_participation(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<(), WorkspacesError> {
    sync_add_to_channels(pool, workspace_id, user_id).await
}

/// Heal every channel seat of every workspace `user_id` belongs to. Called
/// on gateway identify so a join/create race never strands a member out of
/// its own replay.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
pub async fn ensure_all_participation(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<(), WorkspacesError> {
    for (workspace, _) in list_workspaces(pool, user_id).await? {
        ensure_channel_participation(pool, workspace.id, user_id).await?;
    }
    Ok(())
}

/// Drop `user_id`'s member-targeted channel overrides in `workspace_id`.
/// Called on kick/ban/leave so a rejoining user never resurrects stale
/// per-member grants or denials.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
async fn drop_member_overrides<'c, E>(
    ex: E,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<(), WorkspacesError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "DELETE FROM channel_overrides
         WHERE target_kind = 'member' AND target = $2 AND channel_id IN (
             SELECT id FROM channels WHERE workspace_id = $1
         )",
    )
    .bind(workspace_id)
    .bind(user_id.to_string())
    .execute(ex)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Copy `user_id` into every channel conversation of `workspace_id`.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
async fn sync_add_to_channels<'c, E>(
    ex: E,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<(), WorkspacesError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO conversation_participants (conversation_id, user_id)
         SELECT conversation_id, $2 FROM channels WHERE workspace_id = $1
         ON CONFLICT DO NOTHING",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(ex)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Drop `user_id` from every channel conversation of `workspace_id`.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
async fn sync_remove_from_channels<'c, E>(
    ex: E,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<(), WorkspacesError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "DELETE FROM conversation_participants
         WHERE user_id = $2 AND conversation_id IN (
             SELECT conversation_id FROM channels WHERE workspace_id = $1
         )",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(ex)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Create a workspace with the caller as owner.
///
/// # Errors
///
/// Returns [`WorkspacesError::BadInput`] for a bad name, or
/// [`WorkspacesError::Database`] on database failure.
pub async fn create_workspace(
    pool: &sqlx::PgPool,
    owner_id: Uuid,
    name: &str,
) -> Result<Workspace, WorkspacesError> {
    let name = clean_name(name)?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let id = Uuid::now_v7();
    let row: WorkspaceRow = sqlx::query_as(
        "INSERT INTO workspaces (id, name, owner_id) VALUES ($1, $2, $3)
         RETURNING id, name, owner_id, created_at, updated_at",
    )
    .bind(id)
    .bind(&name)
    .bind(owner_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(id)
    .bind(owner_id)
    .execute(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    record_audit(
        &mut *tx,
        id,
        owner_id,
        "workspace.created",
        None,
        &serde_json::json!({ "name": name }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(to_workspace(row))
}

/// Workspaces `user_id` belongs to, newest first.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
pub async fn list_workspaces(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<(Workspace, Role)>, WorkspacesError> {
    let rows: Vec<(WorkspaceRow, String)> = sqlx::query_as(
        "SELECT w.id, w.name, w.owner_id, w.created_at, w.updated_at, m.role
         FROM workspaces w JOIN workspace_members m ON m.workspace_id = w.id
         WHERE m.user_id = $1 ORDER BY w.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    rows.into_iter()
        .map(|(row, role)| {
            parse_role(&role)
                .map(|parsed| (to_workspace(row), parsed))
                .map_err(|_| WorkspacesError::Database(sqlx::Error::RowNotFound))
        })
        .collect()
}

/// Fetch one workspace. Member-only; non-members see [`WorkspacesError::NotMember`].
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`] for non-members (and missing rows),
/// [`WorkspacesError::Banned`] for banned members, or
/// [`WorkspacesError::Database`] on database failure.
pub async fn get_workspace(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    workspace_id: Uuid,
) -> Result<(Workspace, Role), WorkspacesError> {
    // Ban first (see `require`): banned callers are ex-members.
    if is_banned(pool, workspace_id, user_id).await? {
        return Err(WorkspacesError::Banned);
    }
    let role = role_of(pool, workspace_id, user_id)
        .await?
        .ok_or(WorkspacesError::NotMember)?;
    let row: WorkspaceRow = sqlx::query_as(
        "SELECT id, name, owner_id, created_at, updated_at FROM workspaces WHERE id = $1",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
    .map_err(WorkspacesError::Database)?
    .ok_or(WorkspacesError::NotMember)?;
    Ok((to_workspace(row), role))
}

/// Rename a workspace. Requires [`Permission::ManageWorkspace`].
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Banned`],
/// [`WorkspacesError::Forbidden`], [`WorkspacesError::BadInput`], or
/// [`WorkspacesError::Database`].
pub async fn rename_workspace(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
) -> Result<Workspace, WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::ManageWorkspace).await?;
    let name = clean_name(name)?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let row: WorkspaceRow = sqlx::query_as(
        "UPDATE workspaces SET name = $2 WHERE id = $1
         RETURNING id, name, owner_id, created_at, updated_at",
    )
    .bind(workspace_id)
    .bind(&name)
    .fetch_optional(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?
    .ok_or(WorkspacesError::NotMember)?;
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "workspace.renamed",
        None,
        &serde_json::json!({ "name": name }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(to_workspace(row))
}

/// Add `target_id` to the workspace with `role`. Requires
/// [`Permission::ManageMembers`]; the rank rule applies to the grant, and
/// banned users cannot be re-added until unbanned.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Banned`] (target
/// is banned), [`WorkspacesError::Forbidden`] (rank or permission),
/// [`WorkspacesError::BadInput`] (already a member — rejoin is a no-op the
/// caller should not mistake for a grant), or [`WorkspacesError::Database`].
pub async fn add_member(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    target_id: Uuid,
    role: Role,
) -> Result<(), WorkspacesError> {
    let actor = require(pool, workspace_id, actor_id, Permission::ManageMembers).await?;
    check_rank(actor, Role::Guest, Some(role))?;
    if role_of(pool, workspace_id, target_id).await?.is_some() {
        return Err(WorkspacesError::BadInput(
            "user is already a member".to_owned(),
        ));
    }
    if is_banned(pool, workspace_id, target_id).await? {
        return Err(WorkspacesError::Banned);
    }
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(workspace_id)
        .bind(target_id)
        .bind(role.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                // Lost a concurrent double-add race: report the duplicate,
                // not a 500.
                WorkspacesError::BadInput("user is already a member".to_owned())
            } else {
                WorkspacesError::Database(err)
            }
        })?;
    sync_add_to_channels(&mut *tx, workspace_id, target_id).await?;
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "member.added",
        Some(target_id),
        &serde_json::json!({ "role": role.as_str() }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Change a member's role. Requires [`Permission::ManageRoles`]; the rank rule
/// applies to both the old and new role, and the last owner cannot be demoted.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`] (actor or target outside the
/// workspace), [`WorkspacesError::Forbidden`] (rank or permission),
/// [`WorkspacesError::BadInput`] (last-owner demotion), or
/// [`WorkspacesError::Database`].
pub async fn set_role(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    target_id: Uuid,
    new_role: Role,
) -> Result<(), WorkspacesError> {
    // Permission first (nice errors), then everything else under lock: the
    // rank check re-reads both rows `FOR UPDATE`, so a promotion racing a
    // kick cannot slip through, and the last-owner guard holds in-tx.
    // (The fresh in-tx actor role below is authoritative; this call only
    // gates on the pre-transaction permission for error quality.)
    require(pool, workspace_id, actor_id, Permission::ManageRoles).await?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let (actor_opt, target_opt) = locked_roles(&mut *tx, workspace_id, actor_id, target_id).await?;
    let actor = actor_opt.ok_or(WorkspacesError::NotMember)?;
    let current = target_opt.ok_or(WorkspacesError::NotMember)?;
    // The pre-transaction permission stands only if the actor still outranks;
    // re-check on fresh rows (a demotion racing this call must win).
    check_rank(actor, current, Some(new_role))?;
    if current == Role::Owner
        && new_role != Role::Owner
        && owner_count(&mut *tx, workspace_id).await? < 2
    {
        return Err(WorkspacesError::BadInput(
            "workspace must keep at least one owner".to_owned(),
        ));
    }
    let updated = sqlx::query(
        "UPDATE workspace_members SET role = $3 WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(target_id)
    .bind(new_role.as_str())
    .execute(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?
    .rows_affected();
    if updated == 0 {
        // Kicked between the lock read and the write (or a concurrent path
        // removed the row): report absence, not success.
        return Err(WorkspacesError::NotMember);
    }
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "member.role_changed",
        Some(target_id),
        &serde_json::json!({ "from": current.as_str(), "to": new_role.as_str() }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Kick a member out. Requires [`Permission::KickMembers`]; the rank rule
/// applies, the last owner cannot be removed, and kicked users may rejoin.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`],
/// [`WorkspacesError::BadInput`] (last owner), or [`WorkspacesError::Database`].
pub async fn remove_member(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    target_id: Uuid,
) -> Result<(), WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::KickMembers).await?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let (actor_opt, target_opt) = locked_roles(&mut *tx, workspace_id, actor_id, target_id).await?;
    let actor = actor_opt.ok_or(WorkspacesError::NotMember)?;
    let current = target_opt.ok_or(WorkspacesError::NotMember)?;
    check_rank(actor, current, None)?;
    if current == Role::Owner && owner_count(&mut *tx, workspace_id).await? < 2 {
        return Err(WorkspacesError::BadInput(
            "workspace must keep at least one owner".to_owned(),
        ));
    }
    sqlx::query("DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2")
        .bind(workspace_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await
        .map_err(WorkspacesError::Database)?;
    sync_remove_from_channels(&mut *tx, workspace_id, target_id).await?;
    drop_member_overrides(&mut *tx, workspace_id, target_id).await?;
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "member.kicked",
        Some(target_id),
        &serde_json::json!({ "role": current.as_str() }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Leave a workspace voluntarily. Any member may leave; the last owner
/// cannot (make another owner first). Departure cleans up like a kick:
/// channel seats and member-targeted overrides go with the leaver, who may
/// rejoin later through the normal paths.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Banned`],
/// [`WorkspacesError::BadInput`] (last owner), or
/// [`WorkspacesError::Database`].
pub async fn leave(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    workspace_id: Uuid,
) -> Result<(), WorkspacesError> {
    let current = role_of(pool, workspace_id, user_id)
        .await?
        .ok_or(WorkspacesError::NotMember)?;
    if is_banned(pool, workspace_id, user_id).await? {
        return Err(WorkspacesError::Banned);
    }
    if current == Role::Owner && owner_count(pool, workspace_id).await? < 2 {
        return Err(WorkspacesError::BadInput(
            "workspace must keep at least one owner — make another owner first".to_owned(),
        ));
    }
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    sqlx::query("DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2")
        .bind(workspace_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(WorkspacesError::Database)?;
    sync_remove_from_channels(&mut *tx, workspace_id, user_id).await?;
    drop_member_overrides(&mut *tx, workspace_id, user_id).await?;
    record_audit(
        &mut *tx,
        workspace_id,
        user_id,
        "member.left",
        Some(user_id),
        &serde_json::json!({ "role": current.as_str() }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Ban a member: records the ban, then removes them like a kick. Requires
/// [`Permission::BanMembers`]; the rank rule applies. Banned users cannot
/// rejoin until [`unban`] runs.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`],
/// [`WorkspacesError::BadInput`] (bad reason, last owner), or
/// [`WorkspacesError::Database`].
pub async fn ban_member(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    target_id: Uuid,
    reason: &str,
) -> Result<(), WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::BanMembers).await?;
    let reason = reason.trim().to_owned();
    if reason.chars().count() > 500 {
        return Err(WorkspacesError::BadInput(
            "reason must be at most 500 characters".to_owned(),
        ));
    }
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let (actor_opt, target_opt) = locked_roles(&mut *tx, workspace_id, actor_id, target_id).await?;
    let actor = actor_opt.ok_or(WorkspacesError::NotMember)?;
    let current = target_opt.ok_or(WorkspacesError::NotMember)?;
    check_rank(actor, current, None)?;
    if current == Role::Owner && owner_count(&mut *tx, workspace_id).await? < 2 {
        return Err(WorkspacesError::BadInput(
            "workspace must keep at least one owner".to_owned(),
        ));
    }
    sqlx::query(
        "INSERT INTO workspace_bans (workspace_id, user_id, banned_by, reason)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(workspace_id)
    .bind(target_id)
    .bind(actor_id)
    .bind(&reason)
    .execute(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    sqlx::query("DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2")
        .bind(workspace_id)
        .bind(target_id)
        .execute(&mut *tx)
        .await
        .map_err(WorkspacesError::Database)?;
    sync_remove_from_channels(&mut *tx, workspace_id, target_id).await?;
    drop_member_overrides(&mut *tx, workspace_id, target_id).await?;
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "member.banned",
        Some(target_id),
        &serde_json::json!({ "reason": reason }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Lift a ban. Requires [`Permission::BanMembers`]. Does NOT re-add
/// membership — the user must rejoin explicitly.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`], or
/// [`WorkspacesError::Database`].
pub async fn unban(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    target_id: Uuid,
) -> Result<(), WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::BanMembers).await?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let removed =
        sqlx::query("DELETE FROM workspace_bans WHERE workspace_id = $1 AND user_id = $2")
            .bind(workspace_id)
            .bind(target_id)
            .execute(&mut *tx)
            .await
            .map_err(WorkspacesError::Database)?
            .rows_affected();
    if removed == 0 {
        // Idempotent no-op: no ban existed, so no audit row either.
        return Ok(());
    }
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "member.unbanned",
        Some(target_id),
        &serde_json::json!({}),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Create a text channel with a backing `channel` conversation, enrolling all
/// current members as participants. Requires [`Permission::ManageChannels`].
/// Retries need list-to-reconcile: channel identity is server-generated per
/// attempt (unique on `(workspace_id, name)`), so a retried create after a
/// lost success response reports the name as taken.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`],
/// [`WorkspacesError::BadInput`] (bad name, or the name is taken in this
/// workspace), or [`WorkspacesError::Database`].
pub async fn create_channel(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
) -> Result<Channel, WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::ManageChannels).await?;
    let name = clean_name(name)?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    // Snapshot inside the tx (not before it): a join committing in a
    // pre-tx window would otherwise miss this channel permanently. Residual
    // skew against fully concurrent joins heals via
    // `ensure_channel_participation` on read/write paths.
    let members: Vec<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM workspace_members WHERE workspace_id = $1")
            .bind(workspace_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(WorkspacesError::Database)?;
    let conversation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO conversations (id, kind, created_by) VALUES ($1, 'channel', $2)")
        .bind(conversation_id)
        .bind(actor_id)
        .execute(&mut *tx)
        .await
        .map_err(WorkspacesError::Database)?;
    for member in &members {
        sqlx::query(
            "INSERT INTO conversation_participants (conversation_id, user_id)
             VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(conversation_id)
        .bind(member)
        .execute(&mut *tx)
        .await
        .map_err(WorkspacesError::Database)?;
    }
    let id = Uuid::now_v7();
    let row: ChannelRow = sqlx::query_as(
        "INSERT INTO channels (id, workspace_id, conversation_id, name, created_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, workspace_id, conversation_id, name, kind, created_by, created_at",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(conversation_id)
    .bind(&name)
    .bind(actor_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| {
        if is_unique_violation(&err) {
            WorkspacesError::BadInput("channel name is taken in this workspace".to_owned())
        } else {
            WorkspacesError::Database(err)
        }
    })?;
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "channel.created",
        None,
        &serde_json::json!({ "channel_id": id, "name": name }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(to_channel(row))
}

/// Whether a `sqlx` error is a unique-constraint violation (`23505`).
fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => db.code().is_some_and(|code| code == "23505"),
        _ => false,
    }
}

/// Channels of a workspace, oldest first. Member-only.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Banned`], or
/// [`WorkspacesError::Database`].
pub async fn list_channels(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<Channel>, WorkspacesError> {
    get_workspace(pool, user_id, workspace_id).await?;
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, workspace_id, conversation_id, name, kind, created_by, created_at
         FROM channels WHERE workspace_id = $1 ORDER BY created_at ASC LIMIT $2",
    )
    .bind(workspace_id)
    .bind(MAX_LIST_ROWS)
    .fetch_all(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(rows.into_iter().map(to_channel).collect())
}

/// Fetch one channel. The caller must belong to the parent workspace —
/// otherwise [`WorkspacesError::NotMember`], with no existence oracle.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Banned`], or
/// [`WorkspacesError::Database`].
pub async fn get_channel(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    channel_id: Uuid,
) -> Result<Channel, WorkspacesError> {
    let row: Option<ChannelRow> = sqlx::query_as(
        "SELECT id, workspace_id, conversation_id, name, kind, created_by, created_at
         FROM channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_optional(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    let Some(row) = row else {
        return Err(WorkspacesError::NotMember);
    };
    let channel = to_channel(row);
    // Membership gate on the parent workspace (covers bans too).
    get_workspace(pool, user_id, channel.workspace_id).await?;
    Ok(channel)
}

/// Resolve the channel backed by `conversation_id`, if it is a channel
/// conversation. No membership gate — callers add their own check.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
pub async fn channel_by_conversation(
    pool: &sqlx::PgPool,
    conversation_id: Uuid,
) -> Result<Option<Channel>, WorkspacesError> {
    let row: Option<ChannelRow> = sqlx::query_as(
        "SELECT id, workspace_id, conversation_id, name, kind, created_by, created_at
         FROM channels WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(row.map(to_channel))
}

/// Rename a channel. Requires [`Permission::ManageChannels`] on the parent.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`],
/// [`WorkspacesError::BadInput`] (bad name or name taken), or
/// [`WorkspacesError::Database`].
pub async fn rename_channel(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
    name: &str,
) -> Result<Channel, WorkspacesError> {
    let channel = get_channel(pool, actor_id, channel_id).await?;
    require(
        pool,
        channel.workspace_id,
        actor_id,
        Permission::ManageChannels,
    )
    .await?;
    let name = clean_name(name)?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let row: ChannelRow = sqlx::query_as(
        "UPDATE channels SET name = $2 WHERE id = $1
         RETURNING id, workspace_id, conversation_id, name, kind, created_by, created_at",
    )
    .bind(channel_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| {
        if is_unique_violation(&err) {
            WorkspacesError::BadInput("channel name is taken in this workspace".to_owned())
        } else {
            WorkspacesError::Database(err)
        }
    })?;
    record_audit(
        &mut *tx,
        channel.workspace_id,
        actor_id,
        "channel.renamed",
        None,
        &serde_json::json!({ "channel_id": channel_id, "name": name }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(to_channel(row))
}

/// Delete a channel and its backing conversation (messages included).
/// Requires [`Permission::ManageChannels`] on the parent.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`], or
/// [`WorkspacesError::Database`].
pub async fn delete_channel(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
) -> Result<(), WorkspacesError> {
    let channel = get_channel(pool, actor_id, channel_id).await?;
    require(
        pool,
        channel.workspace_id,
        actor_id,
        Permission::ManageChannels,
    )
    .await?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    // Cascades to messages, participants, and the channel row itself.
    let deleted = sqlx::query("DELETE FROM conversations WHERE id = $1")
        .bind(channel.conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(WorkspacesError::Database)?
        .rows_affected();
    if deleted == 0 {
        // Lost a concurrent double-delete: report absence, audit nothing.
        return Err(WorkspacesError::NotMember);
    }
    // Unpublished outbox rows would otherwise ghost-broadcast a deleted
    // conversation to still-connected clients. Published rows are inert
    // (already fanned out) and stay untouched.
    sqlx::query(
        "DELETE FROM outbox WHERE published_at IS NULL AND payload->>'conversation_id' = $1",
    )
    .bind(channel.conversation_id.to_string())
    .execute(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    record_audit(
        &mut *tx,
        channel.workspace_id,
        actor_id,
        "channel.deleted",
        None,
        &serde_json::json!({ "channel_id": channel_id, "name": channel.name }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Set (upsert) a channel permission override. Only `SEND_MESSAGES` is
/// overridable in this slice: `MANAGE_MESSAGES` has no enforcement point yet
/// (no message-moderation endpoint consumes it), so accepting it would sell
/// a moderation control that does nothing. The management slice will widen
/// this set alongside real enforcement. Role targets must name a real role;
/// member targets must belong to the workspace. Requires
/// [`Permission::ManageChannels`] on the parent.
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`],
/// [`WorkspacesError::BadInput`] (bad target/permission), or
/// [`WorkspacesError::Database`].
#[allow(clippy::too_many_arguments)]
pub async fn set_override(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
    target_kind: &str,
    target: &str,
    permission: Permission,
    allowed: bool,
) -> Result<ChannelOverride, WorkspacesError> {
    if permission != Permission::SendMessages {
        return Err(WorkspacesError::BadInput(
            "only SEND_MESSAGES is overridable until message moderation lands".to_owned(),
        ));
    }
    let channel = get_channel(pool, actor_id, channel_id).await?;
    let actor_role = require(
        pool,
        channel.workspace_id,
        actor_id,
        Permission::ManageChannels,
    )
    .await?;
    let target_kind = target_kind.trim().to_lowercase();
    if target_kind != "role" && target_kind != "member" {
        return Err(WorkspacesError::BadInput(
            "target_kind must be role or member".to_owned(),
        ));
    }
    let (normalized_target, target_role) = if target_kind == "role" {
        let role = parse_role(target)?;
        (role.as_str().to_owned(), role)
    } else {
        let user_id: Uuid = target.trim().parse().map_err(|_: uuid::Error| {
            WorkspacesError::BadInput("member target must be a user id".to_owned())
        })?;
        let role = role_of(pool, channel.workspace_id, user_id).await?.ok_or(
            WorkspacesError::BadInput("override target is not a workspace member".to_owned()),
        )?;
        (user_id.to_string(), role)
    };
    // Overrides are moderation power: the rank rule covers the target, so an
    // admin can neither silence an owner nor favor one past their station.
    check_rank(actor_role, target_role, None)?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    sqlx::query(
        "INSERT INTO channel_overrides (channel_id, target_kind, target, permission, allowed)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (channel_id, target_kind, target, permission)
         DO UPDATE SET allowed = EXCLUDED.allowed",
    )
    .bind(channel_id)
    .bind(&target_kind)
    .bind(&normalized_target)
    .bind(permission.as_str())
    .bind(allowed)
    .execute(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    record_audit(
        &mut *tx,
        channel.workspace_id,
        actor_id,
        "channel.override_set",
        None,
        &serde_json::json!({
            "channel_id": channel_id,
            "target_kind": target_kind,
            "target": normalized_target,
            "permission": permission.as_str(),
            "allowed": allowed,
        }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(ChannelOverride {
        channel_id,
        target_kind,
        target: normalized_target,
        permission: permission.as_str().to_owned(),
        allowed,
    })
}

/// Remove a channel override. Requires [`Permission::ManageChannels`].
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`], or
/// [`WorkspacesError::Database`].
pub async fn delete_override(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
    target_kind: &str,
    target: &str,
    permission: Permission,
) -> Result<(), WorkspacesError> {
    let channel = get_channel(pool, actor_id, channel_id).await?;
    require(
        pool,
        channel.workspace_id,
        actor_id,
        Permission::ManageChannels,
    )
    .await?;
    // Same normalization as `set_override`: raw path segments would otherwise
    // silently match zero rows (`Member` vs `member`, uppercase UUIDs).
    let target_kind = target_kind.trim().to_lowercase();
    if target_kind != "role" && target_kind != "member" {
        return Err(WorkspacesError::BadInput(
            "target_kind must be role or member".to_owned(),
        ));
    }
    let normalized_target = if target_kind == "role" {
        parse_role(target)?.as_str().to_owned()
    } else {
        target
            .trim()
            .parse::<Uuid>()
            .map(|id| id.to_string())
            .map_err(|_: uuid::Error| {
                WorkspacesError::BadInput("member target must be a user id".to_owned())
            })?
    };
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let removed = sqlx::query(
        "DELETE FROM channel_overrides
         WHERE channel_id = $1 AND target_kind = $2 AND target = $3 AND permission = $4",
    )
    .bind(channel_id)
    .bind(&target_kind)
    .bind(&normalized_target)
    .bind(permission.as_str())
    .execute(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?
    .rows_affected();
    if removed == 0 {
        // Idempotent no-op: nothing matched, so no audit row either.
        return Ok(());
    }
    record_audit(
        &mut *tx,
        channel.workspace_id,
        actor_id,
        "channel.override_removed",
        None,
        &serde_json::json!({ "channel_id": channel_id }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Overrides on a channel. Any workspace member may read them (transparency);
/// non-members see [`WorkspacesError::NotMember`].
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Banned`], or
/// [`WorkspacesError::Database`].
pub async fn list_overrides(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    channel_id: Uuid,
) -> Result<Vec<ChannelOverride>, WorkspacesError> {
    let channel = get_channel(pool, user_id, channel_id).await?;
    let rows: Vec<OverrideRow> = sqlx::query_as(
        "SELECT target_kind, target, permission, allowed FROM channel_overrides
         WHERE channel_id = $1",
    )
    .bind(channel.id)
    .fetch_all(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(rows
        .into_iter()
        .map(|row| ChannelOverride {
            channel_id: channel.id,
            target_kind: row.0,
            target: row.1,
            permission: row.2,
            allowed: row.3,
        })
        .collect())
}

/// Evaluate `permission` for `user_id` on a channel: workspace membership +
/// no ban, then the base role grant, then the role override, then the
/// member-specific override (most specific wins).
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure. Unknown
/// channels and non-members evaluate to `false` (no oracle).
pub async fn channel_allowed(
    pool: &sqlx::PgPool,
    channel_id: Uuid,
    user_id: Uuid,
    permission: Permission,
) -> Result<bool, WorkspacesError> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT workspace_id FROM channels WHERE id = $1")
        .bind(channel_id)
        .fetch_optional(pool)
        .await
        .map_err(WorkspacesError::Database)?;
    let Some((workspace_id,)) = row else {
        return Ok(false);
    };
    let Some(role) = role_of(pool, workspace_id, user_id).await? else {
        return Ok(false);
    };
    if is_banned(pool, workspace_id, user_id).await? {
        return Ok(false);
    }
    let mut allowed = role_has(role, permission);
    let role_hit: Option<bool> = sqlx::query_scalar(
        "SELECT allowed FROM channel_overrides
         WHERE channel_id = $1 AND target_kind = 'role' AND target = $2 AND permission = $3",
    )
    .bind(channel_id)
    .bind(role.as_str())
    .bind(permission.as_str())
    .fetch_optional(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    if let Some(hit) = role_hit {
        allowed = hit;
    }
    let member_hit: Option<bool> = sqlx::query_scalar(
        "SELECT allowed FROM channel_overrides
         WHERE channel_id = $1 AND target_kind = 'member' AND target = $2 AND permission = $3",
    )
    .bind(channel_id)
    .bind(user_id.to_string())
    .bind(permission.as_str())
    .fetch_optional(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    if let Some(hit) = member_hit {
        allowed = hit;
    }
    Ok(allowed)
}

/// Whether `user_id` may send in `channel_id`. Used by the message path on
/// top of the participant check.
///
/// # Errors
///
/// Returns [`WorkspacesError::Database`] on database failure.
pub async fn can_send(
    pool: &sqlx::PgPool,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<bool, WorkspacesError> {
    channel_allowed(pool, channel_id, user_id, Permission::SendMessages).await
}

/// Longest invite lifetime: 366 days. Bounds the user-controlled
/// `expires_in_secs` so the deadline arithmetic cannot overflow.
const MAX_INVITE_TTL_SECS: i64 = 366 * 24 * 60 * 60;

/// Cap for unbounded list endpoints (channels, invites). Reads stay bounded
/// until cursor pagination lands; the cap is documented on each query.
const MAX_LIST_ROWS: i64 = 500;

/// Fresh random invite code (144 bits, URL-safe, 24 chars). The RNG failure
/// mode is a panic inside `OsRng`, matching the password module's stance that
/// a broken OS RNG is unrecoverable.
fn new_invite_code() -> String {
    use base64::Engine as _;
    use rand_core::RngCore as _;
    let mut bytes = [0u8; 18];
    rand_core::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Create an invite. Requires [`Permission::ManageMembers`]; the initial role
/// must sit below the actor's rank (owner bypasses).
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`],
/// [`WorkspacesError::BadInput`] (bad role/expiry/uses), or
/// [`WorkspacesError::Database`].
pub async fn create_invite(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    initial_role: Role,
    expires_in_secs: Option<i64>,
    max_uses: Option<i32>,
) -> Result<Invite, WorkspacesError> {
    let actor = require(pool, workspace_id, actor_id, Permission::ManageMembers).await?;
    check_rank(actor, Role::Guest, Some(initial_role))?;
    if initial_role == Role::Owner {
        // Owner rank is never handed out by invite in Alpha; founders add
        // owners directly. Keeps a leaked invite from forging ownership.
        return Err(WorkspacesError::BadInput(
            "invites cannot grant the owner role".to_owned(),
        ));
    }
    if max_uses.is_some_and(|n| n <= 0) {
        return Err(WorkspacesError::BadInput(
            "max_uses must be positive".to_owned(),
        ));
    }
    if expires_in_secs.is_some_and(|n| n <= 0 || n > MAX_INVITE_TTL_SECS) {
        return Err(WorkspacesError::BadInput(
            "expires_in_secs must be between 1 and 31622400 (366 days)".to_owned(),
        ));
    }
    let expires_at = expires_in_secs.map(|secs| Utc::now() + chrono::Duration::seconds(secs));
    let id = Uuid::now_v7();
    let code = new_invite_code();
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let row: InviteRow = sqlx::query_as(
        "INSERT INTO invites (id, workspace_id, code, created_by, initial_role, expires_at, max_uses)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, workspace_id, code, created_by, initial_role, expires_at,
                    max_uses, uses, revoked, created_at",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(&code)
    .bind(actor_id)
    .bind(initial_role.as_str())
    .bind(expires_at)
    .bind(max_uses)
    .fetch_one(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "invite.created",
        None,
        &serde_json::json!({ "invite_id": id, "initial_role": initial_role.as_str() }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(to_invite(row))
}

/// Invites of a workspace, newest first. Requires
/// [`Permission::ManageMembers`] (codes are bearer credentials — members at
/// large must not list them).
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`], or
/// [`WorkspacesError::Database`].
pub async fn list_invites(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<Invite>, WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::ManageMembers).await?;
    let rows: Vec<InviteRow> = sqlx::query_as(
        "SELECT id, workspace_id, code, created_by, initial_role, expires_at,
                max_uses, uses, revoked, created_at
         FROM invites WHERE workspace_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(workspace_id)
    .bind(MAX_LIST_ROWS)
    .fetch_all(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(rows.into_iter().map(to_invite).collect())
}

/// Revoke an invite. Requires [`Permission::ManageMembers`].
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`], or
/// [`WorkspacesError::Database`].
pub async fn revoke_invite(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    invite_id: Uuid,
) -> Result<(), WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::ManageMembers).await?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let removed =
        sqlx::query("UPDATE invites SET revoked = TRUE WHERE id = $1 AND workspace_id = $2")
            .bind(invite_id)
            .bind(workspace_id)
            .execute(&mut *tx)
            .await
            .map_err(WorkspacesError::Database)?
            .rows_affected();
    if removed == 0 {
        // Unknown id, or an id from another workspace: same wall, no oracle,
        // and no audit row for something that never happened.
        return Err(WorkspacesError::NotMember);
    }
    record_audit(
        &mut *tx,
        workspace_id,
        actor_id,
        "invite.revoked",
        None,
        &serde_json::json!({ "invite_id": invite_id }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(())
}

/// Whether an invite row is still redeemable.
fn invite_usable(row: &InviteRow, now: DateTime<Utc>) -> bool {
    if row.8 {
        return false;
    }
    if row.5.is_some_and(|deadline| deadline <= now) {
        return false;
    }
    if row.6.is_some_and(|cap| row.7 >= cap) {
        return false;
    }
    true
}

/// Redeem an invite code. Already-members get their current seat back without
/// burning a use; banned users stay out. All failure modes collapse to
/// [`WorkspacesError::InviteRejected`] (no code-validity oracle) except the
/// ban, which the banned caller already knows about.
///
/// # Errors
///
/// Returns [`WorkspacesError::InviteRejected`], [`WorkspacesError::Banned`],
/// or [`WorkspacesError::Database`].
pub async fn join_via_invite(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    code: &str,
) -> Result<(Workspace, Role), WorkspacesError> {
    let code = code.trim().to_owned();
    if code.is_empty() {
        return Err(WorkspacesError::InviteRejected);
    }
    let row: InviteRow = sqlx::query_as(
        "SELECT id, workspace_id, code, created_by, initial_role, expires_at,
                max_uses, uses, revoked, created_at
         FROM invites WHERE code = $1",
    )
    .bind(&code)
    .fetch_optional(pool)
    .await
    .map_err(WorkspacesError::Database)?
    .ok_or(WorkspacesError::InviteRejected)?;
    let workspace_id = row.1;
    if is_banned(pool, workspace_id, user_id).await? {
        return Err(WorkspacesError::Banned);
    }
    if let Some(current) = role_of(pool, workspace_id, user_id).await? {
        let workspace = get_workspace(pool, user_id, workspace_id).await?.0;
        return Ok((workspace, current));
    }
    if !invite_usable(&row, Utc::now()) {
        return Err(WorkspacesError::InviteRejected);
    }
    let initial_role =
        parse_role(&row.4).map_err(|_| WorkspacesError::Database(sqlx::Error::RowNotFound))?;
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    // Re-check under the row lock: the cap may have been hit concurrently.
    let fresh: InviteRow = sqlx::query_as(
        "SELECT id, workspace_id, code, created_by, initial_role, expires_at,
                max_uses, uses, revoked, created_at
         FROM invites WHERE id = $1 FOR UPDATE",
    )
    .bind(row.0)
    .fetch_one(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    if !invite_usable(&fresh, Utc::now()) {
        return Err(WorkspacesError::InviteRejected);
    }
    // The INSERT is the arbiter: under the membership primary key it inserts
    // exactly once per user, so `inserted == 0` means the seat was already
    // taken — by an earlier add or by a concurrent join winner.
    let inserted = sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(workspace_id)
    .bind(user_id)
    .bind(initial_role.as_str())
    .execute(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?
    .rows_affected();
    if inserted == 0 {
        // Already seated, or a concurrent join won the race to this INSERT:
        // either way the seat is not ours. Roll back (nothing of ours
        // is in the tx), burn no use, audit nothing, and return the actual
        // current seat rather than the invite's initial role.
        tx.rollback().await.map_err(WorkspacesError::Database)?;
        let raw: String = sqlx::query_scalar(
            "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(workspace_id)
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(|err| match err {
            sqlx::Error::RowNotFound => WorkspacesError::NotMember,
            other => WorkspacesError::Database(other),
        })?;
        let role =
            parse_role(&raw).map_err(|_| WorkspacesError::Database(sqlx::Error::RowNotFound))?;
        let workspace = get_workspace(pool, user_id, workspace_id).await?.0;
        return Ok((workspace, role));
    }
    // Fresh seat (possibly ours from a lost race — either way the
    // row now exists exactly once, so exactly one use is charged).
    sqlx::query("UPDATE invites SET uses = uses + 1 WHERE id = $1")
        .bind(row.0)
        .execute(&mut *tx)
        .await
        .map_err(WorkspacesError::Database)?;
    sync_add_to_channels(&mut *tx, workspace_id, user_id).await?;
    record_audit(
        &mut *tx,
        workspace_id,
        user_id,
        "invite.accepted",
        Some(user_id),
        &serde_json::json!({ "invite_id": row.0 }),
    )
    .await?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    let workspace = get_workspace(pool, user_id, workspace_id).await?.0;
    Ok((workspace, initial_role))
}

/// Recent audit entries, newest first. Requires [`Permission::ViewAuditLog`].
///
/// # Errors
///
/// Returns [`WorkspacesError::NotMember`], [`WorkspacesError::Forbidden`], or
/// [`WorkspacesError::Database`].
pub async fn list_audit(
    pool: &sqlx::PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    limit: i64,
) -> Result<Vec<AuditEntry>, WorkspacesError> {
    require(pool, workspace_id, actor_id, Permission::ViewAuditLog).await?;
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT id, workspace_id, actor_id, action, target_id, detail, created_at
             FROM workspace_audit WHERE workspace_id = $1
             ORDER BY created_at DESC LIMIT $2",
    )
    .bind(workspace_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await
    .map_err(WorkspacesError::Database)?;
    Ok(rows
        .into_iter()
        .map(|row| AuditEntry {
            id: row.0,
            workspace_id: row.1,
            actor_id: row.2,
            action: row.3,
            target_id: row.4,
            detail: row.5,
            created_at: row.6,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The matrix is the product's trust core: owner/admin hold everything,
    /// moderators get the safety subset, members send, guests observe.
    #[test]
    fn permission_matrix() {
        use Permission as P;
        use Role as R;
        for perm in [
            P::ManageWorkspace,
            P::ManageChannels,
            P::ManageRoles,
            P::ManageMembers,
            P::KickMembers,
            P::BanMembers,
            P::SendMessages,
            P::ManageMessages,
            P::ViewAuditLog,
        ] {
            assert!(role_has(R::Owner, perm), "owner lacks {perm:?}");
            assert!(role_has(R::Admin, perm), "admin lacks {perm:?}");
        }
        assert!(role_has(Role::Moderator, Permission::KickMembers));
        assert!(role_has(Role::Moderator, Permission::ManageMembers));
        assert!(role_has(Role::Moderator, Permission::ManageMessages));
        assert!(role_has(Role::Moderator, Permission::ViewAuditLog));
        assert!(!role_has(Role::Moderator, Permission::BanMembers));
        assert!(!role_has(Role::Moderator, Permission::ManageRoles));
        assert!(role_has(Role::Member, Permission::SendMessages));
        assert!(!role_has(Role::Member, Permission::ManageMessages));
        assert!(!role_has(Role::Member, Permission::KickMembers));
        for perm in [
            P::ManageWorkspace,
            P::ManageChannels,
            P::ManageRoles,
            P::ManageMembers,
            P::KickMembers,
            P::BanMembers,
            P::SendMessages,
            P::ManageMessages,
            P::ViewAuditLog,
        ] {
            assert!(!role_has(Role::Guest, perm), "guest holds {perm:?}");
        }
    }

    /// Rank rule: peers and superiors are untouchable; grants cannot escalate
    /// to (or past) the actor's own rank; the owner bypasses everything.
    #[test]
    fn rank_rule() {
        // Moderator over member/guest works, peer+ fails.
        assert!(check_rank(Role::Moderator, Role::Member, None).is_ok());
        assert!(check_rank(Role::Moderator, Role::Moderator, None).is_err());
        assert!(check_rank(Role::Moderator, Role::Guest, Some(Role::Member)).is_ok());
        assert!(check_rank(Role::Moderator, Role::Guest, Some(Role::Moderator)).is_err());
        // Admin cannot touch admins or mint them.
        assert!(check_rank(Role::Admin, Role::Admin, None).is_err());
        assert!(check_rank(Role::Admin, Role::Member, Some(Role::Admin)).is_err());
        assert!(check_rank(Role::Admin, Role::Member, Some(Role::Moderator)).is_ok());
        // Owner is unbounded.
        assert!(check_rank(Role::Owner, Role::Owner, Some(Role::Owner)).is_ok());
    }

    /// The invite gate is a pure predicate: revoked, expired, and spent codes
    /// all fail; everything else passes.
    #[test]
    fn invite_usable_boundaries() {
        let now = Utc::now();
        let base: InviteRow = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            "code".to_owned(),
            Uuid::now_v7(),
            "member".to_owned(),
            None,
            None,
            0,
            false,
            now,
        );
        assert!(invite_usable(&base, now));
        let mut revoked = base.clone();
        revoked.8 = true;
        assert!(!invite_usable(&revoked, now));
        let mut expired = base.clone();
        expired.5 = Some(now - chrono::Duration::seconds(1));
        assert!(!invite_usable(&expired, now));
        let mut fresh = base.clone();
        fresh.5 = Some(now + chrono::Duration::seconds(60));
        assert!(invite_usable(&fresh, now));
        let mut spent = base.clone();
        spent.6 = Some(1);
        spent.7 = 1;
        assert!(!invite_usable(&spent, now));
        let mut room = base.clone();
        room.6 = Some(2);
        room.7 = 1;
        assert!(invite_usable(&room, now));
    }

    /// Parsing is forgiving about case/whitespace and strict about values.
    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(
            parse_role("  MoDeRaToR ").expect("case-insensitive"),
            Role::Moderator
        );
        assert!(parse_role("superadmin").is_err());
        assert!(parse_role("").is_err());
        assert_eq!(
            parse_permission("send_messages").expect("case-insensitive"),
            Permission::SendMessages
        );
        assert!(parse_permission("DROP_TABLES").is_err());
    }

    /// Count audit entries (test helper; keeps long lifecycle tests under the
    /// pedantic line budget).
    async fn audit_len(pool: &sqlx::PgPool, actor: Uuid, workspace: Uuid) -> usize {
        list_audit(pool, actor, workspace, 100)
            .await
            .expect("audit")
            .len()
    }

    /// Register one user for workspace tests, returning its id.
    async fn user(pool: &sqlx::PgPool, stamp: i64, name: &str) -> Uuid {
        crate::auth::create_user(
            pool,
            &format!("{name}{stamp}"),
            &format!("{name}{stamp}@example.com"),
            name,
            "pw-workspace-1",
        )
        .await
        .expect("register user")
        .id
    }

    /// Requires a live database; skips honestly without one. Membership and
    /// rank discipline: adds, duplicate refusal, moderator limits, role
    /// changes, and the last-owner guard.
    #[tokio::test]
    async fn workspace_roles_and_ranks() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_roles_and_ranks (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w1owner").await;
        let moddy = user(&pool, stamp, "w1mod").await;
        let member = user(&pool, stamp, "w1member").await;
        let guest = user(&pool, stamp, "w1guest").await;
        let outsider = user(&pool, stamp, "w1out").await;

        let workspace = create_workspace(&pool, owner, "  Test Space  ")
            .await
            .expect("create workspace");
        assert_eq!(workspace.name, "Test Space");
        assert!(create_workspace(&pool, owner, "   ").await.is_err());

        // Outsider sees nothing (no oracle), guest sees nothing before invite.
        assert!(matches!(
            get_workspace(&pool, outsider, workspace.id).await,
            Err(WorkspacesError::NotMember)
        ));

        add_member(&pool, owner, workspace.id, moddy, Role::Moderator)
            .await
            .expect("add moderator");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        // Duplicate add is a loud no-op refusal, not a silent grant.
        assert!(add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .is_err());
        // Moderator cannot mint admins or touch the owner.
        assert!(add_member(&pool, moddy, workspace.id, guest, Role::Admin)
            .await
            .is_err());
        assert!(remove_member(&pool, moddy, workspace.id, owner)
            .await
            .is_err());
        add_member(&pool, moddy, workspace.id, guest, Role::Guest)
            .await
            .expect("moderator adds guest");

        // Role changes: admin path and the last-owner guard.
        set_role(&pool, owner, workspace.id, guest, Role::Member)
            .await
            .expect("promote guest");
        assert!(
            set_role(&pool, moddy, workspace.id, member, Role::Moderator)
                .await
                .is_err()
        );
        assert!(set_role(&pool, owner, workspace.id, owner, Role::Member)
            .await
            .is_err());
        // The last-owner guard covers kick and ban too, not just demotion.
        assert!(remove_member(&pool, owner, workspace.id, owner)
            .await
            .is_err());
        assert!(ban_member(&pool, owner, workspace.id, owner, "self")
            .await
            .is_err());
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Channels and
    /// overrides: creation gates, the send matrix, and override precedence
    /// (member-specific beats role beats the base grant).
    #[tokio::test]
    async fn channel_overrides_eval() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: channel_overrides_eval (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w2owner").await;
        let member = user(&pool, stamp, "w2member").await;
        let guest = user(&pool, stamp, "w2guest").await;
        let outsider = user(&pool, stamp, "w2out").await;

        let workspace = create_workspace(&pool, owner, "Override Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        add_member(&pool, owner, workspace.id, guest, Role::Guest)
            .await
            .expect("add guest");

        // Channels: create, list, rename, duplicate-name refusal.
        let general = create_channel(&pool, owner, workspace.id, "general")
            .await
            .expect("create channel");
        assert_eq!(general.kind, "text");
        assert!(create_channel(&pool, member, workspace.id, "nope")
            .await
            .is_err());
        assert!(create_channel(&pool, owner, workspace.id, "general")
            .await
            .is_err());
        let channels = list_channels(&pool, member, workspace.id)
            .await
            .expect("list channels");
        assert_eq!(channels.len(), 1);
        assert!(list_channels(&pool, outsider, workspace.id).await.is_err());

        // Permission core: members send, guests do not — until overridden.
        assert!(can_send(&pool, general.id, member).await.expect("eval"));
        assert!(!can_send(&pool, general.id, guest).await.expect("eval"));
        set_override(
            &pool,
            owner,
            general.id,
            "role",
            "member",
            Permission::SendMessages,
            false,
        )
        .await
        .expect("deny members");
        assert!(!can_send(&pool, general.id, member).await.expect("eval"));
        // Member-specific allow beats the role deny.
        set_override(
            &pool,
            owner,
            general.id,
            "member",
            &member.to_string(),
            Permission::SendMessages,
            true,
        )
        .await
        .expect("allow one member");
        assert!(can_send(&pool, general.id, member).await.expect("eval"));
        let overrides = list_overrides(&pool, member, general.id)
            .await
            .expect("list overrides");
        assert_eq!(overrides.len(), 2);
        let audits_before = audit_len(&pool, owner, workspace.id).await;
        delete_override(
            &pool,
            owner,
            general.id,
            "role",
            "member",
            Permission::SendMessages,
        )
        .await
        .expect("remove role deny");
        // A repeat delete is a silent no-op: still Ok, and no second audit row.
        delete_override(
            &pool,
            owner,
            general.id,
            "role",
            "member",
            Permission::SendMessages,
        )
        .await
        .expect("repeat delete stays silent");
        assert_eq!(
            audit_len(&pool, owner, workspace.id).await,
            audits_before + 1,
            "exactly one audit row for one removal"
        );
        rename_channel(&pool, owner, general.id, "lobby")
            .await
            .expect("rename channel");
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Invites:
    /// bounded single-use codes, idempotent rejoin without burning uses,
    /// redeem-as-member keeps the current seat, revocation, and rejection.
    #[tokio::test]
    async fn invites_accept_revoke() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: invites_accept_revoke (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w3owner").await;
        let member = user(&pool, stamp, "w3member").await;
        let guest = user(&pool, stamp, "w3guest").await;
        let outsider = user(&pool, stamp, "w3out").await;
        let late = user(&pool, stamp, "w3late").await;

        let workspace = create_workspace(&pool, owner, "Invite Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        add_member(&pool, owner, workspace.id, guest, Role::Guest)
            .await
            .expect("add guest");

        // Invites: bounded single-use code, accept, revoke, reject.
        let invite = create_invite(&pool, owner, workspace.id, Role::Member, None, Some(1))
            .await
            .expect("create invite");
        assert!(!invite.code.is_empty());
        assert!(
            create_invite(&pool, owner, workspace.id, Role::Owner, None, None)
                .await
                .is_err()
        );
        let (joined, granted) = join_via_invite(&pool, outsider, &invite.code)
            .await
            .expect("accept invite");
        assert_eq!(joined.id, workspace.id);
        assert_eq!(granted, Role::Member);
        // Single use is spent.
        assert!(join_via_invite(&pool, late, &invite.code).await.is_err());
        // Re-joining as a member does not burn uses and returns the seat.
        let (_, again) = join_via_invite(&pool, outsider, &invite.code)
            .await
            .expect("idempotent rejoin");
        assert_eq!(again, Role::Member);
        let spent: i32 = sqlx::query_scalar("SELECT uses FROM invites WHERE id = $1")
            .bind(invite.id)
            .fetch_one(&pool)
            .await
            .expect("read uses");
        assert_eq!(spent, 1, "idempotent rejoin must not burn a use");
        let multi = create_invite(&pool, owner, workspace.id, Role::Guest, None, None)
            .await
            .expect("open invite");
        revoke_invite(&pool, owner, workspace.id, multi.id)
            .await
            .expect("revoke");
        assert!(join_via_invite(&pool, late, &multi.code).await.is_err());
        assert!(join_via_invite(&pool, late, "not-a-real-code")
            .await
            .is_err());
        // A member added out-of-band who then redeems a live code keeps the
        // seat they have and burns no uses (the lost-join-race shape).
        let open = create_invite(&pool, owner, workspace.id, Role::Guest, None, None)
            .await
            .expect("open invite");
        add_member(&pool, owner, workspace.id, late, Role::Member)
            .await
            .expect("add late directly");
        let (_, seat) = join_via_invite(&pool, late, &open.code)
            .await
            .expect("redeem as member");
        assert_eq!(seat, Role::Member);
        let burned: i32 = sqlx::query_scalar("SELECT uses FROM invites WHERE id = $1")
            .bind(open.id)
            .fetch_one(&pool)
            .await
            .expect("read uses");
        assert_eq!(burned, 0, "redeem-as-member must not burn a use");
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Kicks, bans,
    /// and the audit trail: removal syncs channel seats, bans persist and
    /// block re-adds, unbans heal, and audit entries stay payload-free.
    #[tokio::test]
    async fn kick_ban_audit_delete() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: kick_ban_audit_delete (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w3bowner").await;
        let member = user(&pool, stamp, "w3bmember").await;
        let guest = user(&pool, stamp, "w3bguest").await;
        let outsider = user(&pool, stamp, "w3bout").await;

        let workspace = create_workspace(&pool, owner, "Ban Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        add_member(&pool, owner, workspace.id, guest, Role::Guest)
            .await
            .expect("add guest");
        // The kick target needs a seat first so removal has something to sync.
        add_member(&pool, owner, workspace.id, outsider, Role::Member)
            .await
            .expect("add outsider");
        let general = create_channel(&pool, owner, workspace.id, "general")
            .await
            .expect("create channel");

        // Kick: out, channel seats gone, may rejoin. Ban: out and stays out.
        remove_member(&pool, owner, workspace.id, outsider)
            .await
            .expect("kick");
        assert!(get_workspace(&pool, outsider, workspace.id).await.is_err());
        ban_member(&pool, owner, workspace.id, member, "spam")
            .await
            .expect("ban");
        assert!(matches!(
            get_workspace(&pool, member, workspace.id).await,
            Err(WorkspacesError::Banned)
        ));
        assert!(!can_send(&pool, general.id, member).await.expect("eval"));
        assert!(add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .is_err());
        unban(&pool, owner, workspace.id, member)
            .await
            .expect("unban");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("re-add after unban");

        // Audit covers the lifecycle and carries no envelope bytes.
        let audit = list_audit(&pool, owner, workspace.id, 100)
            .await
            .expect("audit");
        assert!(audit.len() >= 8, "audit has {} entries", audit.len());
        for entry in &audit {
            let rendered = serde_json::to_string(&entry.detail).expect("render");
            assert!(!rendered.contains("ciphertext"), "audit leaks payload");
        }
        // The schema is pinned per action, not just scanned for secrets:
        // member.added carries exactly the granted role.
        let added = audit
            .iter()
            .find(|entry| entry.action == "member.added")
            .expect("member.added audited");
        let keys: Vec<&str> = added
            .detail
            .as_object()
            .expect("detail is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["role"]);
        assert!(list_audit(&pool, guest, workspace.id, 10).await.is_err());

        // Channel deletion removes the backing conversation too.
        delete_channel(&pool, owner, general.id)
            .await
            .expect("delete channel");
        let gone: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conversations WHERE id = $1)")
                .bind(general.conversation_id)
                .fetch_one(&pool)
                .await
                .expect("probe conversation");
        assert!(!gone, "backing conversation must go with the channel");
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Overrides are
    /// moderation power: the rank rule covers the target, so an admin can
    /// silence a member but neither silence nor favor an owner.
    #[tokio::test]
    async fn override_rank_rule() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: override_rank_rule (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w4owner").await;
        let admin = user(&pool, stamp, "w4admin").await;
        let member = user(&pool, stamp, "w4member").await;

        let workspace = create_workspace(&pool, owner, "Rank Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, admin, Role::Admin)
            .await
            .expect("add admin");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        let channel = create_channel(&pool, owner, workspace.id, "mod")
            .await
            .expect("create channel");

        // An admin may silence a member...
        set_override(
            &pool,
            admin,
            channel.id,
            "member",
            &member.to_string(),
            Permission::SendMessages,
            false,
        )
        .await
        .expect("admin silences member");
        // ...but may not touch the owner, by id or by role, deny or allow.
        assert!(matches!(
            set_override(
                &pool,
                admin,
                channel.id,
                "member",
                &owner.to_string(),
                Permission::SendMessages,
                false,
            )
            .await,
            Err(WorkspacesError::Forbidden)
        ));
        assert!(matches!(
            set_override(
                &pool,
                admin,
                channel.id,
                "role",
                "owner",
                Permission::SendMessages,
                false,
            )
            .await,
            Err(WorkspacesError::Forbidden)
        ));
        assert!(matches!(
            set_override(
                &pool,
                admin,
                channel.id,
                "role",
                "owner",
                Permission::SendMessages,
                true,
            )
            .await,
            Err(WorkspacesError::Forbidden)
        ));
        // The owner bypasses the rank rule entirely.
        set_override(
            &pool,
            owner,
            channel.id,
            "member",
            &member.to_string(),
            Permission::SendMessages,
            false,
        )
        .await
        .expect("owner override always lands");
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Revoking an
    /// unknown id — or one from another workspace — reports absence without
    /// an oracle and leaves the foreign invite usable.
    #[tokio::test]
    async fn revoke_unknown_is_not_member() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: revoke_unknown_is_not_member (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w5owner").await;
        let stranger = user(&pool, stamp, "w5stranger").await;

        let first = create_workspace(&pool, owner, "First Space")
            .await
            .expect("create workspace");
        assert!(matches!(
            revoke_invite(&pool, owner, first.id, Uuid::now_v7()).await,
            Err(WorkspacesError::NotMember)
        ));
        let second = create_workspace(&pool, owner, "Second Space")
            .await
            .expect("create workspace");
        let foreign = create_invite(&pool, owner, second.id, Role::Member, None, None)
            .await
            .expect("foreign invite");
        assert!(matches!(
            revoke_invite(&pool, owner, first.id, foreign.id).await,
            Err(WorkspacesError::NotMember)
        ));
        // The attempt revoked nothing: the foreign invite still redeems.
        let (joined, _) = join_via_invite(&pool, stranger, &foreign.code)
            .await
            .expect("foreign invite intact");
        assert_eq!(joined.id, second.id);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Deleting a
    /// channel twice reports absence the second time, and unpublished outbox
    /// rows for the deleted conversation are cleaned so clients never
    /// ghost-receive a dead channel.
    #[tokio::test]
    async fn channel_delete_cleans_outbox() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: channel_delete_cleans_outbox (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w6owner").await;
        let member = user(&pool, stamp, "w6member").await;

        let workspace = create_workspace(&pool, owner, "Delete Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        let channel = create_channel(&pool, owner, workspace.id, "doomed")
            .await
            .expect("create channel");
        let (_, created) = crate::messaging::send_message(
            &pool,
            member,
            channel.conversation_id,
            Uuid::now_v7(),
            b"bye",
            None,
        )
        .await
        .expect("send");
        assert!(created);

        delete_channel(&pool, owner, channel.id)
            .await
            .expect("delete channel");
        assert!(matches!(
            delete_channel(&pool, owner, channel.id).await,
            Err(WorkspacesError::NotMember)
        ));
        let claimed = crate::messaging::claim_outbox(&pool, 100)
            .await
            .expect("claim");
        for entry in &claimed {
            let convo = entry
                .payload
                .get("conversation_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            assert_ne!(
                convo,
                channel.conversation_id.to_string(),
                "deleted channel must not ghost-broadcast"
            );
        }
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. History is a
    /// read: guests and send-revoked members can read it, outsiders cannot.
    #[tokio::test]
    async fn guest_reads_history() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: guest_reads_history (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w7owner").await;
        let member = user(&pool, stamp, "w7member").await;
        let guest = user(&pool, stamp, "w7guest").await;
        let outsider = user(&pool, stamp, "w7out").await;

        let workspace = create_workspace(&pool, owner, "Read Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        add_member(&pool, owner, workspace.id, guest, Role::Guest)
            .await
            .expect("add guest");
        let channel = create_channel(&pool, owner, workspace.id, "general")
            .await
            .expect("create channel");
        crate::messaging::send_message(
            &pool,
            member,
            channel.conversation_id,
            Uuid::now_v7(),
            b"hi",
            None,
        )
        .await
        .expect("send");

        // Guests hold no SEND grant but read fine...
        let seen = crate::messaging::message_history(&pool, guest, channel.conversation_id, 0, 50)
            .await
            .expect("guest reads");
        assert_eq!(seen.len(), 1);
        // ...as do members whose SEND grant was revoked...
        set_override(
            &pool,
            owner,
            channel.id,
            "role",
            "member",
            Permission::SendMessages,
            false,
        )
        .await
        .expect("mute members");
        assert!(!can_send(&pool, channel.id, member).await.expect("eval"));
        let seen_muted =
            crate::messaging::message_history(&pool, member, channel.conversation_id, 0, 50)
                .await
                .expect("muted member still reads");
        assert_eq!(seen_muted.len(), 1);
        // ...while outsiders still hit the wall.
        assert!(
            crate::messaging::message_history(&pool, outsider, channel.conversation_id, 0, 50)
                .await
                .is_err()
        );
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. List endpoints
    /// stay bounded: past the cap the oldest-first page is truncated.
    #[tokio::test]
    async fn channel_list_is_bounded() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: channel_list_is_bounded (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "w8owner").await;
        let workspace = create_workspace(&pool, owner, "Cap Space")
            .await
            .expect("create workspace");
        for index in 0..505 {
            create_channel(&pool, owner, workspace.id, &format!("cap{index:03}"))
                .await
                .expect("create channel");
        }
        let channels = list_channels(&pool, owner, workspace.id)
            .await
            .expect("list channels");
        assert_eq!(channels.len(), 500);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Channel messages
    /// flow through the Milestone C path: synced members send and read,
    /// kicked/banned writers are locked out without burning sequence.
    #[tokio::test]
    async fn channel_send_history() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: channel_send_history (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "chowner").await;
        let writer = user(&pool, stamp, "chwriter").await;
        let lurker = user(&pool, stamp, "chlurker").await;

        let workspace = create_workspace(&pool, owner, "Channel Flow")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, writer, Role::Member)
            .await
            .expect("add writer");
        let channel = create_channel(&pool, owner, workspace.id, "announcements")
            .await
            .expect("create channel");

        // Writer (synced into the backing conversation) sends; owner reads.
        let key = Uuid::now_v7();
        let (first, created) = crate::messaging::send_message(
            &pool,
            writer,
            channel.conversation_id,
            key,
            b"channel-bro",
            None,
        )
        .await
        .expect("channel send");
        assert!(created);
        assert_eq!(first.seq, 1);
        let history =
            crate::messaging::message_history(&pool, owner, channel.conversation_id, 0, 50)
                .await
                .expect("owner history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].ciphertext, b"channel-bro");
        // Outsider hits the wall.
        assert!(
            crate::messaging::message_history(&pool, lurker, channel.conversation_id, 0, 50)
                .await
                .is_err()
        );

        // Kick revokes the writer's seat mid-channel.
        remove_member(&pool, owner, workspace.id, writer)
            .await
            .expect("kick writer");
        assert!(crate::messaging::send_message(
            &pool,
            writer,
            channel.conversation_id,
            Uuid::now_v7(),
            b"after-kick",
            None,
        )
        .await
        .is_err());
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The trigger is
    /// the last-owner backstop behind the application guard: raw demotion or
    /// deletion of a sole owner aborts, even bypassing every app check.
    #[tokio::test]
    async fn owner_trigger_backstop() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: owner_trigger_backstop (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "towner").await;
        let workspace = create_workspace(&pool, owner, "Trigger Space")
            .await
            .expect("create workspace");

        // Sole-owner demotion and deletion both abort at the database.
        assert!(sqlx::query(
            "UPDATE workspace_members SET role = 'member'
             WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(workspace.id)
        .bind(owner)
        .execute(&pool)
        .await
        .is_err());
        assert!(sqlx::query(
            "DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(workspace.id)
        .bind(owner)
        .execute(&pool)
        .await
        .is_err());
        // A second owner makes the same statements legal again.
        let spare = user(&pool, stamp, "tspare").await;
        add_member(&pool, owner, workspace.id, spare, Role::Owner)
            .await
            .expect("second owner");
        sqlx::query(
            "UPDATE workspace_members SET role = 'member'
             WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(workspace.id)
        .bind(owner)
        .execute(&pool)
        .await
        .expect("demote with spare owner");
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Two owners race
    /// to demote each other: ordered row locks serialize the pair, so the
    /// loser meets the guard instead of deadlocking or orphaning the space.
    #[tokio::test]
    async fn concurrent_managers_keep_an_owner() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: concurrent_managers_keep_an_owner (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let first = user(&pool, stamp, "cfirst").await;
        let second = user(&pool, stamp, "csecond").await;
        let workspace = create_workspace(&pool, first, "Race Space")
            .await
            .expect("create workspace");
        add_member(&pool, first, workspace.id, second, Role::Owner)
            .await
            .expect("second owner");

        let (left, right) = tokio::join!(
            set_role(&pool, first, workspace.id, second, Role::Member),
            set_role(&pool, second, workspace.id, first, Role::Member),
        );
        // Exactly one demotion can win: the loser meets the guard (app-level
        // or trigger), never a deadlock, never zero owners.
        assert!(left.is_ok() ^ right.is_ok(), "one demotion wins");
        assert!(owner_count(&pool, workspace.id).await.expect("count") >= 1);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Voluntary exit:
    /// members leave cleanly (seats and overrides go with them), the last
    /// owner is stopped, and a second owner frees the first to go.
    #[tokio::test]
    async fn leave_flow() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: leave_flow (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "lowner").await;
        let member = user(&pool, stamp, "lmember").await;
        let workspace = create_workspace(&pool, owner, "Leave Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");

        leave(&pool, member, workspace.id)
            .await
            .expect("member leaves");
        assert!(get_workspace(&pool, member, workspace.id).await.is_err());
        assert!(leave(&pool, owner, workspace.id).await.is_err());

        let spare = user(&pool, stamp, "lspare").await;
        add_member(&pool, owner, workspace.id, spare, Role::Owner)
            .await
            .expect("second owner");
        leave(&pool, owner, workspace.id)
            .await
            .expect("first owner leaves");
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Kicks reset
    /// channel privilege: a member-specific grant does not survive the kick,
    /// so rejoining restores the base role default instead of the stale row.
    #[tokio::test]
    async fn override_resurrection() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: override_resurrection (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "rowner").await;
        let member = user(&pool, stamp, "rmember").await;
        let workspace = create_workspace(&pool, owner, "Resurrect Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        let channel = create_channel(&pool, owner, workspace.id, "general")
            .await
            .expect("create channel");

        set_override(
            &pool,
            owner,
            channel.id,
            "member",
            &member.to_string(),
            Permission::SendMessages,
            false,
        )
        .await
        .expect("deny member");
        assert!(!can_send(&pool, channel.id, member).await.expect("eval"));
        remove_member(&pool, owner, workspace.id, member)
            .await
            .expect("kick");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("rejoin");
        assert!(list_overrides(&pool, member, channel.id)
            .await
            .expect("overrides")
            .is_empty());
        assert!(can_send(&pool, channel.id, member).await.expect("eval"));
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Invite lifetimes
    /// are bounded: zero, negative, and overflow-scale TTLs are refused
    /// before any date arithmetic runs; an hour works.
    #[tokio::test]
    async fn invite_expiry_bounds() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: invite_expiry_bounds (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "eowner").await;
        let workspace = create_workspace(&pool, owner, "Expiry Space")
            .await
            .expect("create workspace");

        for ttl in [0, -5, i64::MAX, 366 * 24 * 60 * 60 + 1] {
            assert!(
                create_invite(&pool, owner, workspace.id, Role::Member, Some(ttl), None)
                    .await
                    .is_err(),
                "ttl {ttl} must be refused"
            );
        }
        let invite = create_invite(&pool, owner, workspace.id, Role::Member, Some(3600), None)
            .await
            .expect("hour-long invite");
        assert!(invite.expires_at.is_some());
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Override
    /// hygiene: `MANAGE_MESSAGES` has no enforcement yet so it is refused
    /// outright; deletes normalize like sets (uppercase UUID works); and an
    /// unban with no ban behind it writes no audit row.
    #[tokio::test]
    async fn override_hygiene() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: override_hygiene (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "howner").await;
        let member = user(&pool, stamp, "hmember").await;
        let workspace = create_workspace(&pool, owner, "Hygiene Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        let channel = create_channel(&pool, owner, workspace.id, "general")
            .await
            .expect("create channel");

        assert!(set_override(
            &pool,
            owner,
            channel.id,
            "role",
            "member",
            Permission::ManageMessages,
            false,
        )
        .await
        .is_err());
        set_override(
            &pool,
            owner,
            channel.id,
            "member",
            &member.to_string(),
            Permission::SendMessages,
            false,
        )
        .await
        .expect("deny member");
        delete_override(
            &pool,
            owner,
            channel.id,
            "MEMBER",
            &member.to_string().to_uppercase(),
            Permission::SendMessages,
        )
        .await
        .expect("normalized delete");
        assert!(list_overrides(&pool, member, channel.id)
            .await
            .expect("overrides")
            .is_empty());

        let before = list_audit(&pool, owner, workspace.id, 100)
            .await
            .expect("audit")
            .len();
        unban(&pool, owner, workspace.id, member)
            .await
            .expect("unban no-op");
        let after = list_audit(&pool, owner, workspace.id, 100)
            .await
            .expect("audit")
            .len();
        assert_eq!(before, after, "no-op unban writes no audit");
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The
    /// join/create-channel race heals: a member stranded out of a channel's
    /// participants is re-seated by the ensure call read paths run.
    #[tokio::test]
    async fn participation_heal() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: participation_heal (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let owner = user(&pool, stamp, "powner").await;
        let member = user(&pool, stamp, "pmember").await;
        let workspace = create_workspace(&pool, owner, "Heal Space")
            .await
            .expect("create workspace");
        add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .expect("add member");
        let channel = create_channel(&pool, owner, workspace.id, "general")
            .await
            .expect("create channel");

        // Simulate the race: member exists, channel seat does not.
        sqlx::query(
            "DELETE FROM conversation_participants
             WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(channel.conversation_id)
        .bind(member)
        .execute(&pool)
        .await
        .expect("strand member");
        assert!(
            !crate::messaging::is_member(&pool, channel.conversation_id, member)
                .await
                .expect("probe")
        );
        ensure_channel_participation(&pool, workspace.id, member)
            .await
            .expect("heal");
        assert!(
            crate::messaging::is_member(&pool, channel.conversation_id, member)
                .await
                .expect("probe")
        );
        pool.close().await;
    }
}
