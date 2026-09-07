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
//! Isolation: this branch has no workspaces module, so isolation is
//! per-user/per-conversation and the query endpoints serve only the requesting
//! user's own rollup. Workspace scoping is a follow-up once workspaces land.

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
    if !crate::messaging::is_member(pool, conversation_id, caller_id)
        .await
        .map_err(StatsError::from)?
    {
        return Err(StatsError::NotMember);
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

/// Fold every outbox row after `last_seen` (exclusive), advancing it past each
/// row visited — including skipped ones, so poison never blocks the cursor.
/// Returns rows newly counted.
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
        "SELECT id, topic, payload FROM outbox
         WHERE (CAST($1 AS UUID) IS NULL OR id > CAST($1 AS UUID))
         ORDER BY id ASC LIMIT $2",
    )
    .bind(*last_seen)
    .bind(STATS_BATCH)
    .fetch_all(pool)
    .await
    .map_err(StatsError::Database)?;
    let mut folded = 0_u64;
    for (id, topic, payload) in &rows {
        // The cursor advances only past attempted rows: successes, duplicates,
        // and warned skips. A database error returns WITHOUT advancing, so the
        // next round retries the same row instead of dropping it and every row
        // fetched after it.
        match process_event(pool, *id, topic, payload).await {
            Ok(true) => folded += 1,
            Ok(false) => {}
            Err(StatsError::BadInput(detail)) => {
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
             WHERE (payload->>'conversation_id')::uuid = $1
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(conversation)
        .fetch_one(pool)
        .await
        .expect("outbox row")
    }

    /// Requires a live database; skips honestly without one. One send folds
    /// to exactly one count, globally and per-conversation.
    #[tokio::test]
    async fn send_counts_once() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: send_counts_once (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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

    /// Requires a live database; skips honestly without one. The same event
    /// delivered twice still counts once; the retry reports `false`.
    #[tokio::test]
    async fn duplicate_delivery_is_idempotent() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: duplicate_delivery_is_idempotent (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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

    /// Requires a live database; skips honestly without one. A retry storm —
    /// the same event 25 times — still counts exactly once.
    #[tokio::test]
    async fn retry_storm_counts_once() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: retry_storm_counts_once (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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

    /// Requires a live database; skips honestly without one. Ten concurrent
    /// deliveries of the same event count exactly once.
    #[tokio::test]
    async fn concurrent_duplicates_count_once() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: concurrent_duplicates_count_once (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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

    /// Requires a live database; skips honestly without one. Counts stay with
    /// their sender and conversation: no cross-user leakage, and outsiders
    /// cannot probe conversations they are not in.
    #[tokio::test]
    async fn per_user_and_conversation_isolation() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: per_user_and_conversation_isolation (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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

    /// Requires a live database; skips honestly without one. Stats tables
    /// hold ids, counts, and timestamps only — no content columns and no
    /// content values — and the consumed payload itself carries no content.
    #[tokio::test]
    async fn stats_store_no_plaintext() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: stats_store_no_plaintext (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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
             WHERE table_name IN ('user_message_stats', 'user_conversation_stats', 'stats_processed_events')",
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
                    "processed_at"
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

    /// Requires a live database; skips honestly without one. The worker fold
    /// counts real events, warns past a malformed row without stalling (a
    /// later message still counts), and a second round recounts nothing.
    /// Assertions are per-user deltas, so parallel tests sharing the database
    /// cannot flake them.
    #[tokio::test]
    async fn fold_skips_poison_and_advances() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: fold_skips_poison_and_advances (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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

    /// Requires a live database; skips honestly without one. Foreign topics
    /// are ignored and malformed payloads are rejected without counting.
    #[tokio::test]
    async fn foreign_and_malformed_events_do_not_count() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: foreign_and_malformed_events_do_not_count (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

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
