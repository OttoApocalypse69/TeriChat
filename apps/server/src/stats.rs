//! Issue #7 Stats baseline: per-user message counters folded from outbox
//! routing metadata — never plaintext.
//!
//! Privacy shape: the consumer reads only `actor_id`/`conversation_id` from
//! `message.created` outbox payloads. Those payloads carry no ciphertext (they
//! hold `event_id`, `event_type`, `version`, `timestamp`, `actor_id`,
//! `conversation_id`, and `data` with `message_id`/`seq` — verified against
//! [`crate::messaging::send_message`]). Stats tables store ids, counts, and
//! timestamps only.
//!
//! Idempotency: every consumed event is recorded once in
//! `stats_processed_events` (`PRIMARY KEY` on `event_id`); concurrent
//! duplicate deliveries race on that insert and exactly one wins the count,
//! so outbox retries and worker restarts are safe.
//!
//! Isolation: endpoints return only the caller's own counters. Channel scope
//! uses authoritative workspace read membership (including guests), not stale
//! transport participation. DMs/groups require conversation membership.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Outbox topic folded into counters. Any other topic is ignored, never an error.
const COUNTABLE_TOPIC: &str = "message.created";

/// Worker poll batch cap per round.
const STATS_BATCH: i64 = 200;

/// Typed stats error.
#[derive(Debug)]
pub enum StatsError {
    /// Caller is not a conversation member (also covers missing rows).
    NotMember,
    /// Caller-supplied value rejected (malformed event payload).
    BadInput(String),
    /// Database failure (logged by the HTTP layer, never shown verbatim).
    Database(sqlx::Error),
}

impl std::fmt::Display for StatsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMember => write!(f, "not a conversation member"),
            Self::BadInput(detail) => write!(f, "{detail}"),
            Self::Database(_) => write!(f, "database error"),
        }
    }
}

impl std::error::Error for StatsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(err) => Some(err),
            _ => None,
        }
    }
}

impl From<crate::messaging::MessagingError> for StatsError {
    fn from(err: crate::messaging::MessagingError) -> Self {
        match err {
            crate::messaging::MessagingError::NotMember => Self::NotMember,
            crate::messaging::MessagingError::EmptyCiphertext
            | crate::messaging::MessagingError::CiphertextTooLarge => {
                Self::BadInput(err.to_string())
            }
            crate::messaging::MessagingError::Database(inner) => Self::Database(inner),
        }
    }
}

/// Per-account activity rollup.
#[derive(Debug, Clone)]
pub struct UserStats {
    /// Account id.
    pub user_id: Uuid,
    /// Lifetime sent messages (all conversations).
    pub message_count: i64,
    /// Last send time, if any.
    pub last_message_at: Option<DateTime<Utc>>,
}

/// The caller's own counter inside one conversation.
#[derive(Debug, Clone)]
pub struct ConversationStats {
    /// Account id (always the requesting user).
    pub user_id: Uuid,
    /// Conversation the count is scoped to.
    pub conversation_id: Uuid,
    /// Sent messages in that conversation.
    pub message_count: i64,
    /// Last send time in that conversation, if any.
    pub last_message_at: Option<DateTime<Utc>>,
}

/// Fold one outbox event into the counters. Returns `true` when the event was
/// newly counted, `false` for duplicates (same `event_id`) and for ignored
/// topics. Malformed `message.created` payloads are [`StatsError::BadInput`]
/// so the worker can skip past them with a warning.
///
/// # Errors
///
/// Returns [`StatsError::BadInput`] on a malformed `message.created` payload,
/// or [`StatsError::Database`] on database failure.
pub async fn process_event(
    pool: &sqlx::PgPool,
    event_id: Uuid,
    topic: &str,
    payload: &serde_json::Value,
) -> Result<bool, StatsError> {
    if topic != COUNTABLE_TOPIC {
        return Ok(false);
    }
    let (actor_id, conversation_id) = event_ids(payload)?;
    record_message(pool, event_id, actor_id, conversation_id).await
}

/// Extract (`actor_id`, `conversation_id`) from a `message.created` payload.
///
/// # Errors
///
/// Returns [`StatsError::BadInput`] when either field is missing or unparsable.
fn event_ids(payload: &serde_json::Value) -> Result<(Uuid, Uuid), StatsError> {
    let bad =
        |what: &str| StatsError::BadInput(format!("message.created payload missing valid {what}"));
    let actor = payload
        .get("actor_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| bad("actor_id"))?;
    let conversation = payload
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| bad("conversation_id"))?;
    let actor_id = actor.parse().map_err(|_| bad("actor_id"))?;
    let conversation_id = conversation.parse().map_err(|_| bad("conversation_id"))?;
    Ok((actor_id, conversation_id))
}

/// Record one counted message for `actor_id` in `conversation_id`, deduped on
/// `event_id`. The dedupe insert and both counter upserts commit atomically;
/// concurrent duplicates race on the `PRIMARY KEY` and exactly one counts.
/// Returns `true` when newly counted, `false` for a duplicate.
///
/// # Errors
///
/// Returns [`StatsError::BadInput`] when the ids reference no known
/// user/conversation, or [`StatsError::Database`] on database failure.
pub async fn record_message(
    pool: &sqlx::PgPool,
    event_id: Uuid,
    actor_id: Uuid,
    conversation_id: Uuid,
) -> Result<bool, StatsError> {
    let mut tx = pool.begin().await.map_err(StatsError::Database)?;
    // Dedupe first: concurrent duplicates race here, exactly one wins.
    let claimed = sqlx::query(
        "INSERT INTO stats_processed_events (event_id, user_id, conversation_id)
         VALUES ($1, $2, $3) ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(actor_id)
    .bind(conversation_id)
    .execute(&mut *tx)
    .await
    .map_err(map_unknown_scope)?;
    if claimed.rows_affected() == 0 {
        tx.commit().await.map_err(StatsError::Database)?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO user_message_stats (user_id, message_count, last_message_at)
         VALUES ($1, 1, now())
         ON CONFLICT (user_id) DO UPDATE
         SET message_count = user_message_stats.message_count + 1,
             last_message_at = now()",
    )
    .bind(actor_id)
    .execute(&mut *tx)
    .await
    .map_err(StatsError::Database)?;
    sqlx::query(
        "INSERT INTO user_conversation_stats (user_id, conversation_id, message_count, last_message_at)
         VALUES ($1, $2, 1, now())
         ON CONFLICT (user_id, conversation_id) DO UPDATE
         SET message_count = user_conversation_stats.message_count + 1,
             last_message_at = now()",
    )
    .bind(actor_id)
    .bind(conversation_id)
    .execute(&mut *tx)
    .await
    .map_err(StatsError::Database)?;
    tx.commit().await.map_err(StatsError::Database)?;
    Ok(true)
}

/// Map a foreign-key violation (event references a user/conversation the
/// database no longer knows) to [`StatsError::BadInput`]; anything else stays
/// a database error.
fn map_unknown_scope(err: sqlx::Error) -> StatsError {
    if let sqlx::Error::Database(db_err) = &err {
        if db_err.code().as_deref() == Some("23503") {
            return StatsError::BadInput(
                "event references an unknown user or conversation".to_owned(),
            );
        }
    }
    StatsError::Database(err)
}

/// The user's own rollup. First-time users get a zeroed rollup, not an error.
///
/// # Errors
///
/// Returns [`StatsError::Database`] on database failure.
pub async fn own_stats(pool: &sqlx::PgPool, user_id: Uuid) -> Result<UserStats, StatsError> {
    let row: Option<(i64, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT message_count, last_message_at FROM user_message_stats WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(StatsError::Database)?;
    Ok(UserStats {
        user_id,
        message_count: row.map_or(0, |(count, _)| count),
        last_message_at: row.and_then(|(_, at)| at),
    })
}

/// The caller's own counter inside one conversation (zeroed when the caller
/// never sent there). Membership-checked: non-members see
/// [`StatsError::NotMember`], so callers cannot probe conversations they are
/// not in.
///
/// # Errors
///
/// Returns [`StatsError::NotMember`] for non-members (or missing rows), or
/// [`StatsError::Database`] on database failure.
pub async fn conversation_stats(
    pool: &sqlx::PgPool,
    caller_id: Uuid,
    conversation_id: Uuid,
) -> Result<ConversationStats, StatsError> {
    let map_scope = |err| match err {
        crate::workspaces::WorkspacesError::Database(inner) => StatsError::Database(inner),
        _ => StatsError::NotMember,
    };
    match crate::workspaces::channel_by_conversation(pool, conversation_id)
        .await
        .map_err(map_scope)?
    {
        Some(channel) => {
            // Read access is workspace-wide, including guests. SEND grants
            // never gate counters; stale transport participation is not auth.
            crate::workspaces::get_workspace(pool, caller_id, channel.workspace_id)
                .await
                .map_err(map_scope)?;
        }
        None => {
            if !crate::messaging::is_member(pool, conversation_id, caller_id)
                .await
                .map_err(StatsError::from)?
            {
                return Err(StatsError::NotMember);
            }
        }
    }
    let row: Option<(i64, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT message_count, last_message_at FROM user_conversation_stats
         WHERE user_id = $1 AND conversation_id = $2",
    )
    .bind(caller_id)
    .bind(conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(StatsError::Database)?;
    Ok(ConversationStats {
        user_id: caller_id,
        conversation_id,
        message_count: row.map_or(0, |(count, _)| count),
        last_message_at: row.and_then(|(_, at)| at),
    })
}

/// Background stats consumer: fold new outbox rows, then sleep. Restart-safe
/// by construction — a crash replays rows the next boot, and replays dedupe
/// on `event_id`, so every message counts exactly once.
pub async fn stats_worker(pool: sqlx::PgPool) {
    let mut last_seen: Option<Uuid> = None;
    loop {
        match fold_new_events(&pool, &mut last_seen).await {
            Err(err) => tracing::error!("stats fold failed: {err}"),
            Ok(0) => {}
            Ok(folded) => tracing::debug!("stats folded {folded} events"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Fold a bounded batch without a commit-order watermark. UUIDs allocated by
/// uncommitted transactions can arrive below any previously seen id. Durable
/// success/rejection receipts, not the diagnostic `last_seen`, select work.
/// Irrelevant topics are filtered before LIMIT; poison receives a content-free
/// rejection receipt so neither class can starve later valid records.
///
/// # Errors
///
/// Returns [`StatsError::Database`] on database failure. Malformed payloads
/// are skipped with a warning, never an error.
async fn fold_new_events(
    pool: &sqlx::PgPool,
    last_seen: &mut Option<Uuid>,
) -> Result<u64, StatsError> {
    let rows: Vec<(Uuid, String, serde_json::Value)> = sqlx::query_as(
        "SELECT o.id, o.topic, o.payload FROM outbox o
         WHERE o.topic = 'message.created'
           AND NOT EXISTS (SELECT 1 FROM stats_processed_events p WHERE p.event_id = o.id)
           AND NOT EXISTS (SELECT 1 FROM stats_rejected_events r WHERE r.event_id = o.id)
         ORDER BY o.id ASC LIMIT $1",
    )
    .bind(STATS_BATCH)
    .fetch_all(pool)
    .await
    .map_err(StatsError::Database)?;
    let mut folded = 0_u64;
    for (id, topic, payload) in &rows {
        // Database failures retain eligibility for retry. Success receipts and
        // counter updates are atomic; rejected payloads receive a separate
        // durable receipt and never count.
        match process_event(pool, *id, topic, payload).await {
            Ok(true) => folded += 1,
            Ok(false) => {}
            Err(StatsError::BadInput(detail)) => {
                sqlx::query("INSERT INTO stats_rejected_events(event_id) VALUES ($1) ON CONFLICT DO NOTHING")
                    .bind(id).execute(pool).await.map_err(StatsError::Database)?;
                tracing::warn!(event_id = %id, "stats skipping malformed event: {detail}");
            }
            Err(err) => return Err(err),
        }
        *last_seen = Some(*id);
    }
    Ok(folded)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn isolated_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .expect("service-enabled regression requires DATABASE_URL");
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("stats_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .unwrap();
        crate::MIGRATOR.run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn reversed_commits_are_not_lost() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap();
        let user = stats_user(&pool, stamp, "late").await;
        let peer = stats_user(&pool, stamp, "latepeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        let low = Uuid::now_v7();
        let high = Uuid::now_v7();
        assert!(low < high);
        let payload = serde_json::json!({"actor_id":user,"conversation_id":dm.id});
        let mut first = pool.begin().await.unwrap();
        let mut second = pool.begin().await.unwrap();
        for (tx, id) in [(&mut first, low), (&mut second, high)] {
            sqlx::query("INSERT INTO outbox(id,topic,payload) VALUES ($1,'message.created',$2)")
                .bind(id)
                .bind(&payload)
                .execute(&mut **tx)
                .await
                .unwrap();
        }
        second.commit().await.unwrap();
        let mut cursor = None;
        // Independent transactions and schema: one poll sees only the high id.
        fold_new_events(&pool, &mut cursor).await.unwrap();
        assert_eq!(own_stats(&pool, user).await.unwrap().message_count, 1);
        first.commit().await.unwrap();
        fold_new_events(&pool, &mut cursor).await.unwrap();
        assert_eq!(
            own_stats(&pool, user).await.unwrap().message_count,
            2,
            "lower UUID committed after the high-water mark must still count"
        );
        fold_new_events(&pool, &mut cursor).await.unwrap();
        assert_eq!(own_stats(&pool, user).await.unwrap().message_count, 2);
        pool.close().await;
    }

    #[tokio::test]
    async fn poison_batch_cannot_starve_valid_events() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap();
        let user = stats_user(&pool, stamp, "poison").await;
        let peer = stats_user(&pool, stamp, "poisonpeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        for _ in 0..201 {
            sqlx::query("INSERT INTO outbox(id,topic,payload) VALUES ($1,'message.created','{}'),($2,'irrelevant','{}')")
                .bind(Uuid::now_v7()).bind(Uuid::now_v7()).execute(&pool).await.unwrap();
        }
        sent_event(&pool, user, dm.id, b"after poison").await;
        let mut cursor = None;
        for _ in 0..2 {
            fold_new_events(&pool, &mut cursor).await.unwrap();
        }
        assert_eq!(
            own_stats(&pool, user).await.unwrap().message_count,
            1,
            "poison batch must drain"
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn workspace_role_channel_isolation_uses_authoritative_membership() {
        use crate::workspaces::{self, Role};
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap();
        let owner = stats_user(&pool, stamp, "scopeowner").await;
        let member = stats_user(&pool, stamp, "scopemember").await;
        let guest = stats_user(&pool, stamp, "scopeguest").await;
        let outsider = stats_user(&pool, stamp, "scopeoutside").await;
        let one = workspaces::create_workspace(&pool, owner, "one")
            .await
            .unwrap();
        let two = workspaces::create_workspace(&pool, outsider, "two")
            .await
            .unwrap();
        workspaces::add_member(&pool, owner, one.id, member, Role::Member)
            .await
            .unwrap();
        workspaces::add_member(&pool, owner, one.id, guest, Role::Guest)
            .await
            .unwrap();
        let a = workspaces::create_channel(&pool, owner, one.id, "a")
            .await
            .unwrap();
        let b = workspaces::create_channel(&pool, owner, one.id, "b")
            .await
            .unwrap();
        let other = workspaces::create_channel(&pool, outsider, two.id, "other")
            .await
            .unwrap();
        for (user, conversation) in [
            (owner, a.conversation_id),
            (member, a.conversation_id),
            (owner, b.conversation_id),
            (outsider, other.conversation_id),
        ] {
            let (id, topic, payload) = sent_event(&pool, user, conversation, b"synthetic").await;
            process_event(&pool, id, &topic, &payload).await.unwrap();
        }
        for (user, count) in [(owner, 1), (member, 1), (guest, 0)] {
            assert_eq!(
                conversation_stats(&pool, user, a.conversation_id)
                    .await
                    .unwrap()
                    .message_count,
                count
            );
        }
        assert_eq!(
            conversation_stats(&pool, member, b.conversation_id)
                .await
                .unwrap()
                .message_count,
            0
        );
        assert!(matches!(
            conversation_stats(&pool, owner, other.conversation_id).await,
            Err(StatsError::NotMember)
        ));
        // Simulate stale participation left by a join/remove race: transport
        // membership must never confer workspace authority.
        sqlx::query(
            "INSERT INTO conversation_participants(conversation_id,user_id) VALUES ($1,$2)",
        )
        .bind(a.conversation_id)
        .bind(outsider)
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            matches!(
                conversation_stats(&pool, outsider, a.conversation_id).await,
                Err(StatsError::NotMember)
            ),
            "stale participation must fail closed"
        );
        // Conversely, a legitimate read-only guest remains authorized while
        // participation healing catches up with a concurrent channel create.
        sqlx::query(
            "DELETE FROM conversation_participants WHERE conversation_id=$1 AND user_id=$2",
        )
        .bind(a.conversation_id)
        .bind(guest)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            conversation_stats(&pool, guest, a.conversation_id)
                .await
                .unwrap()
                .message_count,
            0
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn ban_revokes_stats_access_without_erasing_or_recounting_history() {
        use crate::workspaces::{self, Role};
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap();
        let owner = stats_user(&pool, stamp, "banowner").await;
        let member = stats_user(&pool, stamp, "banmember").await;
        let workspace = workspaces::create_workspace(&pool, owner, "stats lifecycle")
            .await
            .unwrap();
        workspaces::add_member(&pool, owner, workspace.id, member, Role::Member)
            .await
            .unwrap();
        let channel = workspaces::create_channel(&pool, owner, workspace.id, "history")
            .await
            .unwrap();
        let (id, topic, payload) =
            sent_event(&pool, member, channel.conversation_id, b"synthetic").await;
        assert!(process_event(&pool, id, &topic, &payload).await.unwrap());
        workspaces::ban_member(&pool, owner, workspace.id, member, "synthetic test")
            .await
            .unwrap();
        assert!(matches!(
            conversation_stats(&pool, member, channel.conversation_id).await,
            Err(StatsError::NotMember)
        ));
        assert!(!process_event(&pool, id, &topic, &payload).await.unwrap());
        assert_eq!(own_stats(&pool, member).await.unwrap().message_count, 1);
        workspaces::unban(&pool, owner, workspace.id, member)
            .await
            .unwrap();
        // Lifting a ban is not membership restoration.
        assert!(matches!(
            conversation_stats(&pool, member, channel.conversation_id).await,
            Err(StatsError::NotMember)
        ));
        workspaces::add_member(&pool, owner, workspace.id, member, Role::Guest)
            .await
            .unwrap();
        assert_eq!(
            conversation_stats(&pool, member, channel.conversation_id)
                .await
                .unwrap()
                .message_count,
            1
        );
        assert!(!process_event(&pool, id, &topic, &payload).await.unwrap());
        assert_eq!(own_stats(&pool, member).await.unwrap().message_count, 1);
        pool.close().await;
    }

    /// Raw stats rows dumped for the no-plaintext assertion.
    type UserStatsRow = (String, i64, Option<DateTime<Utc>>, DateTime<Utc>);
    /// Raw per-conversation rows dumped for the no-plaintext assertion.
    type ScopedStatsRow = (String, String, i64, Option<DateTime<Utc>>);
    /// Raw processed-event rows dumped for the no-plaintext assertion.
    type ProcessedEventRow = (String, String, String, DateTime<Utc>);

    /// Register one user for stats tests, returning its id.
    async fn stats_user(pool: &sqlx::PgPool, stamp: i64, name: &str) -> Uuid {
        crate::auth::create_user(
            pool,
            &format!("{name}{stamp}"),
            &format!("{name}{stamp}@example.com"),
            name,
            "pw-stats-1",
        )
        .await
        .expect("register user")
        .id
    }

    /// Send one message and return its outbox event (`id`, `topic`, `payload`).
    async fn sent_event(
        pool: &sqlx::PgPool,
        sender: Uuid,
        conversation: Uuid,
        ciphertext: &[u8],
    ) -> (Uuid, String, serde_json::Value) {
        crate::messaging::send_message(
            pool,
            sender,
            conversation,
            Uuid::now_v7(),
            ciphertext,
            None,
        )
        .await
        .expect("send message");
        sqlx::query_as(
            "SELECT id, topic, payload FROM outbox
             WHERE payload->>'conversation_id' = CAST($1 AS UUID)::text
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(conversation)
        .fetch_one(pool)
        .await
        .expect("outbox row")
    }

    /// Requires a live database; fails if unavailable. One send folds
    /// to exactly one count, globally and per-conversation.
    #[tokio::test]
    async fn send_counts_once() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = stats_user(&pool, stamp, "cntone").await;
        let peer = stats_user(&pool, stamp, "cntpeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .expect("open dm");
        let (event_id, topic, payload) = sent_event(&pool, user, dm.id, b"hello").await;

        assert!(process_event(&pool, event_id, &topic, &payload)
            .await
            .expect("fold"));
        let stats = own_stats(&pool, user).await.expect("own stats");
        assert_eq!(stats.message_count, 1);
        assert!(stats.last_message_at.is_some());
        let scoped = conversation_stats(&pool, user, dm.id)
            .await
            .expect("scoped");
        assert_eq!(scoped.message_count, 1);
        pool.close().await;
    }

    /// Requires a live database; fails if unavailable. The same event
    /// delivered twice still counts once; the retry reports `false`.
    #[tokio::test]
    async fn duplicate_delivery_is_idempotent() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = stats_user(&pool, stamp, "dupone").await;
        let peer = stats_user(&pool, stamp, "duppeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .expect("open dm");
        let (event_id, topic, payload) = sent_event(&pool, user, dm.id, b"hi").await;

        assert!(process_event(&pool, event_id, &topic, &payload)
            .await
            .expect("first"));
        assert!(!process_event(&pool, event_id, &topic, &payload)
            .await
            .expect("retry"));
        assert_eq!(
            own_stats(&pool, user).await.expect("stats").message_count,
            1
        );
        pool.close().await;
    }

    /// Requires a live database; fails if unavailable. A retry storm —
    /// the same event 25 times — still counts exactly once.
    #[tokio::test]
    async fn retry_storm_counts_once() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = stats_user(&pool, stamp, "stormone").await;
        let peer = stats_user(&pool, stamp, "stormpeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .expect("open dm");
        let (event_id, topic, payload) = sent_event(&pool, user, dm.id, b"storm").await;

        for _ in 0..25 {
            process_event(&pool, event_id, &topic, &payload)
                .await
                .expect("fold");
        }
        assert_eq!(
            own_stats(&pool, user).await.expect("stats").message_count,
            1
        );
        assert_eq!(
            conversation_stats(&pool, user, dm.id)
                .await
                .expect("scoped")
                .message_count,
            1
        );
        pool.close().await;
    }

    /// Requires a live database; fails if unavailable. Ten concurrent
    /// deliveries of the same event count exactly once.
    #[tokio::test]
    async fn concurrent_duplicates_count_once() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = stats_user(&pool, stamp, "raceone").await;
        let peer = stats_user(&pool, stamp, "racepeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .expect("open dm");
        let (event_id, topic, payload) = sent_event(&pool, user, dm.id, b"race").await;

        let mut handles = Vec::new();
        for _ in 0..10 {
            let (pool, topic, payload) = (pool.clone(), topic.clone(), payload.clone());
            handles.push(tokio::spawn(async move {
                process_event(&pool, event_id, &topic, &payload).await
            }));
        }
        let mut counted = 0;
        for handle in handles {
            if handle.await.expect("task").expect("fold") {
                counted += 1;
            }
        }
        assert_eq!(counted, 1);
        assert_eq!(
            own_stats(&pool, user).await.expect("stats").message_count,
            1
        );
        pool.close().await;
    }

    /// Requires a live database; fails if unavailable. Counts stay with
    /// their sender and conversation: no cross-user leakage, and outsiders
    /// cannot probe conversations they are not in.
    #[tokio::test]
    async fn per_user_and_conversation_isolation() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = stats_user(&pool, stamp, "isoalice").await;
        let bob = stats_user(&pool, stamp, "isobob").await;
        let stranger = stats_user(&pool, stamp, "isostr").await;
        let dm = crate::messaging::find_or_create_dm(&pool, alice, bob)
            .await
            .expect("open dm");
        let group = crate::messaging::create_conversation(&pool, alice, "group", &[bob])
            .await
            .expect("open group");

        // Alice sends in both conversations, Bob only in the DM.
        for (sender, conversation) in [(alice, dm.id), (alice, group.id), (bob, dm.id)] {
            let (event_id, topic, payload) = sent_event(&pool, sender, conversation, b"x").await;
            assert!(process_event(&pool, event_id, &topic, &payload)
                .await
                .expect("fold"));
        }

        assert_eq!(
            own_stats(&pool, alice).await.expect("alice").message_count,
            2
        );
        assert_eq!(own_stats(&pool, bob).await.expect("bob").message_count, 1);
        // Zeroed rollup for a user who never sent.
        assert_eq!(
            own_stats(&pool, stranger)
                .await
                .expect("stranger")
                .message_count,
            0
        );

        // Per-conversation counters only ever hold the caller's own sends.
        assert_eq!(
            conversation_stats(&pool, alice, dm.id)
                .await
                .expect("a/dm")
                .message_count,
            1
        );
        assert_eq!(
            conversation_stats(&pool, bob, dm.id)
                .await
                .expect("b/dm")
                .message_count,
            1
        );
        assert_eq!(
            conversation_stats(&pool, alice, group.id)
                .await
                .expect("a/group")
                .message_count,
            1
        );
        assert_eq!(
            conversation_stats(&pool, bob, group.id)
                .await
                .expect("b/group")
                .message_count,
            0
        );
        // Outsiders cannot probe the conversation at all.
        assert!(matches!(
            conversation_stats(&pool, stranger, dm.id).await,
            Err(StatsError::NotMember)
        ));
        pool.close().await;
    }

    /// Requires a live database; fails if unavailable. Stats tables
    /// hold ids, counts, and timestamps only — no content columns and no
    /// content values — and the consumed payload itself carries no content.
    #[tokio::test]
    async fn stats_store_no_plaintext() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = stats_user(&pool, stamp, "plainone").await;
        let peer = stats_user(&pool, stamp, "plainpeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .expect("open dm");
        let sentinel = format!("sentinel-n0-pl41nt3xt-{stamp}");
        let (event_id, topic, payload) = sent_event(&pool, user, dm.id, sentinel.as_bytes()).await;

        // The consumed payload carries routing metadata only.
        assert!(payload.get("ciphertext").is_none());
        assert!(payload.get("plaintext").is_none());
        assert!(payload.get("body").is_none());
        let payload_text = payload.to_string();
        assert!(!payload_text.contains(&sentinel));

        process_event(&pool, event_id, &topic, &payload)
            .await
            .expect("fold");

        // Only id/count/timestamp columns exist on stats tables.
        let columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT column_name, data_type FROM information_schema.columns
             WHERE table_name IN ('user_message_stats', 'user_conversation_stats', 'stats_processed_events', 'stats_rejected_events')",
        )
        .fetch_all(&pool)
        .await
        .expect("columns");
        assert!(!columns.is_empty());
        for (name, data_type) in &columns {
            assert!(
                [
                    "user_id",
                    "conversation_id",
                    "event_id",
                    "message_count",
                    "last_message_at",
                    "updated_at",
                    "processed_at",
                    "rejected_at"
                ]
                .contains(&name.as_str()),
                "unexpected stats column {name}"
            );
            assert!(
                ["uuid", "bigint", "timestamp with time zone"].contains(&data_type.as_str()),
                "unexpected stats column type {name}: {data_type}"
            );
        }

        // No sentinel bytes anywhere in the stats rows.
        let user_rows: Vec<UserStatsRow> =
            sqlx::query_as("SELECT user_id::text, message_count, last_message_at, updated_at FROM user_message_stats")
                .fetch_all(&pool)
                .await
                .expect("user stats rows");
        let scoped_rows: Vec<ScopedStatsRow> = sqlx::query_as(
            "SELECT user_id::text, conversation_id::text, message_count, last_message_at FROM user_conversation_stats",
        )
        .fetch_all(&pool)
        .await
        .expect("scoped rows");
        let event_rows: Vec<ProcessedEventRow> = sqlx::query_as(
            "SELECT event_id::text, user_id::text, conversation_id::text, processed_at FROM stats_processed_events",
        )
        .fetch_all(&pool)
        .await
        .expect("event rows");
        let dump = format!("{user_rows:?}{scoped_rows:?}{event_rows:?}");
        assert!(!dump.contains(&sentinel));
        pool.close().await;
    }

    /// Requires a live database; fails if unavailable. The worker fold
    /// counts real events, warns past a malformed row without stalling (a
    /// later message still counts), and a second round recounts nothing.
    /// Assertions are per-user deltas, so parallel tests sharing the database
    /// cannot flake them.
    #[tokio::test]
    async fn fold_skips_poison_and_advances() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = stats_user(&pool, stamp, "foldone").await;
        let peer = stats_user(&pool, stamp, "foldpeer").await;
        let dm = crate::messaging::find_or_create_dm(&pool, user, peer)
            .await
            .expect("open dm");
        sent_event(&pool, user, dm.id, b"one").await;
        // Poison: countable topic, unparseable payload. Sits between the two
        // sends, so a stalling cursor would leave the second uncounted.
        sqlx::query("INSERT INTO outbox (id, topic, payload) VALUES ($1, 'message.created', '{}')")
            .bind(Uuid::now_v7())
            .execute(&pool)
            .await
            .expect("plant poison");
        sent_event(&pool, user, dm.id, b"two").await;

        let mut cursor = None;
        let folded = fold_new_events(&pool, &mut cursor).await.expect("fold");
        assert!(
            folded >= 2,
            "fold counted {folded}, want at least our two sends"
        );
        assert!(cursor.is_some(), "cursor must advance");
        let stats = own_stats(&pool, user).await.expect("own stats");
        assert_eq!(stats.message_count, 2);
        // Second round: nothing new for us, cursor holds.
        fold_new_events(&pool, &mut cursor).await.expect("refold");
        let stats = own_stats(&pool, user).await.expect("own stats");
        assert_eq!(stats.message_count, 2);
        pool.close().await;
    }

    /// Requires a live database; fails if unavailable. Foreign topics
    /// are ignored and malformed payloads are rejected without counting.
    #[tokio::test]
    async fn foreign_and_malformed_events_do_not_count() {
        let pool = isolated_pool().await;

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = stats_user(&pool, stamp, "oddone").await;

        assert!(!process_event(
            &pool,
            Uuid::now_v7(),
            "session.revoked",
            &serde_json::json!({})
        )
        .await
        .expect("foreign topic"));
        assert!(process_event(
            &pool,
            Uuid::now_v7(),
            "message.created",
            &serde_json::json!({"nope": 1})
        )
        .await
        .is_err());
        assert_eq!(
            own_stats(&pool, user).await.expect("stats").message_count,
            0
        );
        pool.close().await;
    }
}
