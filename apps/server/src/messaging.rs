//! Milestone C messaging: conversations, opaque envelopes, transactional outbox.
//!
//! Privacy shape: the server routes ciphertext it cannot read. Sends and
//! history are membership-checked; a non-member gets [`MessagingError::NotMember`]
//! whether the conversation exists or not (no existence oracle). Retried sends
//! with the same `client_msg_id` return the original row — no duplicates.
//! Outbox delivery is at-least-once: consumers must dedup by event id.

use chrono::{DateTime, Utc};
use serde_json::json;
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Max envelope bytes the server will store (mirrors the DB `CHECK`).
pub const MAX_CIPHERTEXT_BYTES: usize = 1_048_576;

/// History page cap.
pub const MAX_HISTORY_LIMIT: i64 = 100;

/// Outbox claim batch size per worker round.
pub const OUTBOX_BATCH: i64 = 50;

/// Typed messaging error.
#[derive(Debug)]
pub enum MessagingError {
    /// Caller is not a participant (also covers missing conversations).
    NotMember,
    /// Empty envelope bytes.
    EmptyCiphertext,
    /// Envelope exceeds [`MAX_CIPHERTEXT_BYTES`].
    CiphertextTooLarge,
    /// Database failure (logged by the HTTP layer, never shown verbatim).
    Database(sqlx::Error),
}

impl std::fmt::Display for MessagingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMember => write!(f, "not a conversation member"),
            Self::EmptyCiphertext => write!(f, "ciphertext must not be empty"),
            Self::CiphertextTooLarge => write!(f, "ciphertext too large"),
            Self::Database(_) => write!(f, "database error"),
        }
    }
}

impl std::error::Error for MessagingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(err) => Some(err),
            _ => None,
        }
    }
}

/// Conversation row.
#[derive(Debug, Clone)]
pub struct Conversation {
    /// Conversation id (`UUIDv7`).
    pub id: Uuid,
    /// `dm` or `group`.
    pub kind: String,
    /// Member account ids.
    pub members: Vec<Uuid>,
}

/// Stored message: routing metadata plus opaque bytes. No plaintext here.
#[derive(Debug, Clone)]
pub struct Message {
    /// Message id (`UUIDv7`).
    pub id: Uuid,
    /// Owning conversation.
    pub conversation_id: Uuid,
    /// Sending account.
    pub sender_id: Uuid,
    /// Per-conversation sequence driving history and resume.
    pub seq: i64,
    /// Opaque encrypted payload. Never logged.
    pub ciphertext: Vec<u8>,
    /// Opaque client-supplied nonce, if the suite uses one.
    pub nonce: Option<Vec<u8>>,
    /// Client idempotency key.
    pub client_msg_id: Uuid,
    /// Send time.
    pub sent_at: DateTime<Utc>,
}

/// Claimed outbox row, broadcast to gateway connections.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutboxEntry {
    /// Event id (`UUIDv7`, time-ordered: the global resume position).
    #[serde(rename = "event_id")]
    pub id: Uuid,
    /// Event topic, e.g. `message.created`.
    pub topic: String,
    /// Versioned event payload.
    pub payload: serde_json::Value,
}

/// Raw message row shared by the send and history queries.
type MessageRow = (
    Uuid,
    Uuid,
    Uuid,
    i64,
    Vec<u8>,
    Option<Vec<u8>>,
    Uuid,
    DateTime<Utc>,
);

/// Row-to-domain mapping shared by send and history.
fn to_message(row: MessageRow) -> Message {
    Message {
        id: row.0,
        conversation_id: row.1,
        sender_id: row.2,
        seq: row.3,
        ciphertext: row.4,
        nonce: row.5,
        client_msg_id: row.6,
        sent_at: row.7,
    }
}

/// Create a conversation of `kind` (`dm`/`group`) with `members`. The creator
/// is added when missing. Empty membership is rejected.
pub async fn create_conversation(
    pool: &sqlx::PgPool,
    creator: Uuid,
    kind: &str,
    members: &[Uuid],
) -> Result<Conversation, MessagingError> {
    if kind != "dm" && kind != "group" {
        return Err(MessagingError::Database(sqlx::Error::RowNotFound));
    }
    let mut all: Vec<Uuid> = members.to_vec();
    if !all.contains(&creator) {
        all.push(creator);
    }
    if all.is_empty() {
        return Err(MessagingError::Database(sqlx::Error::RowNotFound));
    }

    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO conversations (id, kind, created_by) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(kind)
        .bind(creator)
        .execute(pool)
        .await
        .map_err(MessagingError::Database)?;
    for member in &all {
        sqlx::query(
            "INSERT INTO conversation_participants (conversation_id, user_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(member)
        .execute(pool)
        .await
        .map_err(MessagingError::Database)?;
    }
    Ok(Conversation {
        id,
        kind: kind.to_owned(),
        members: all,
    })
}

/// Find the `dm` shared by exactly `a` and `b`, or create it. Makes DM
/// creation idempotent: repeated calls return the same conversation.
pub async fn find_or_create_dm(
    pool: &sqlx::PgPool,
    a: Uuid,
    b: Uuid,
) -> Result<Conversation, MessagingError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        r"SELECT c.id FROM conversations c
          JOIN conversation_participants p ON p.conversation_id = c.id
          WHERE c.kind = 'dm' AND p.user_id IN ($1, $2)
          GROUP BY c.id HAVING COUNT(*) = 2
          AND COUNT(*) = (SELECT COUNT(*) FROM conversation_participants WHERE conversation_id = c.id)
          LIMIT 1",
    )
    .bind(a)
    .bind(b)
    .fetch_optional(pool)
    .await
    .map_err(MessagingError::Database)?;

    if let Some(id) = existing {
        let members: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM conversation_participants WHERE conversation_id = $1",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .map_err(MessagingError::Database)?;
        return Ok(Conversation {
            id,
            kind: "dm".to_owned(),
            members,
        });
    }
    create_conversation(pool, a, "dm", &[b]).await
}

/// Membership check. `false` covers both non-members and missing rows.
pub async fn is_member(
    pool: &sqlx::PgPool,
    conversation_id: Uuid,
    user_id: Uuid,
) -> Result<bool, MessagingError> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM conversation_participants WHERE conversation_id = $1 AND user_id = $2)",
    )
    .bind(conversation_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(MessagingError::Database)?;
    Ok(found)
}

/// Send a message: membership-checked, sequenced, and fanned out through the
/// transactional outbox — all in one transaction. Returns the stored message
/// plus whether it was newly created (`false` = idempotent retry returning
/// the original row, without burning a sequence number or event).
pub async fn send_message(
    pool: &sqlx::PgPool,
    sender_id: Uuid,
    conversation_id: Uuid,
    client_msg_id: Uuid,
    ciphertext: &[u8],
    nonce: Option<&[u8]>,
) -> Result<(Message, bool), MessagingError> {
    if ciphertext.is_empty() {
        return Err(MessagingError::EmptyCiphertext);
    }
    if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
        return Err(MessagingError::CiphertextTooLarge);
    }

    let mut tx = pool.begin().await.map_err(MessagingError::Database)?;
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM conversation_participants WHERE conversation_id = $1 AND user_id = $2)",
    )
    .bind(conversation_id)
    .bind(sender_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;
    if !member {
        return Err(MessagingError::NotMember);
    }

    // Idempotent retry first, so a duplicate never consumes a sequence number.
    let existing: Option<MessageRow> = sqlx::query_as(
        r"SELECT id, conversation_id, sender_id, seq, ciphertext, nonce, client_msg_id, sent_at
           FROM messages WHERE conversation_id = $1 AND client_msg_id = $2",
    )
    .bind(conversation_id)
    .bind(client_msg_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;
    if let Some(row) = existing {
        tx.commit().await.map_err(MessagingError::Database)?;
        return Ok((to_message(row), false));
    }

    // Row lock on the conversation serializes concurrent sends for sequencing.
    let seq: i64 = sqlx::query_scalar(
        "UPDATE conversations SET next_seq = next_seq + 1 WHERE id = $1 RETURNING next_seq - 1",
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;

    let message = insert_message(
        &mut tx,
        sender_id,
        conversation_id,
        client_msg_id,
        ciphertext,
        nonce,
        seq,
    )
    .await?;
    tx.commit().await.map_err(MessagingError::Database)?;
    Ok((message, true))
}

/// Insert the sequenced message row plus its outbox event inside the caller's
/// transaction. Split from [`send_message`] to keep each function reviewable.
async fn insert_message(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sender_id: Uuid,
    conversation_id: Uuid,
    client_msg_id: Uuid,
    ciphertext: &[u8],
    nonce: Option<&[u8]>,
    seq: i64,
) -> Result<Message, MessagingError> {
    let message_id = Uuid::now_v7();
    let row: (DateTime<Utc>,) = sqlx::query_as(
        r"INSERT INTO messages (id, conversation_id, sender_id, seq, ciphertext, nonce, client_msg_id)
          VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING sent_at",
    )
    .bind(message_id)
    .bind(conversation_id)
    .bind(sender_id)
    .bind(seq)
    .bind(ciphertext)
    .bind(nonce)
    .bind(client_msg_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(MessagingError::Database)?;

    let event_id = Uuid::now_v7();
    let payload = json!({
        "event_id": event_id,
        "event_type": "message.created",
        "version": 1,
        "timestamp": row.0,
        "actor_id": sender_id,
        "conversation_id": conversation_id,
        "data": { "message_id": message_id, "seq": seq },
    });
    sqlx::query("INSERT INTO outbox (id, topic, payload) VALUES ($1, 'message.created', $2)")
        .bind(event_id)
        .bind(&payload)
        .execute(&mut **tx)
        .await
        .map_err(MessagingError::Database)?;

    Ok(Message {
        id: message_id,
        conversation_id,
        sender_id,
        seq,
        ciphertext: ciphertext.to_vec(),
        nonce: nonce.map(Vec::from),
        client_msg_id,
        sent_at: row.0,
    })
}

/// Message history after `since_seq`, oldest first, capped at
/// [`MAX_HISTORY_LIMIT`]. Membership-checked.
pub async fn message_history(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
    since_seq: i64,
    limit: i64,
) -> Result<Vec<Message>, MessagingError> {
    if !is_member(pool, conversation_id, user_id).await? {
        return Err(MessagingError::NotMember);
    }
    let rows: Vec<MessageRow> = sqlx::query_as(
        r"SELECT id, conversation_id, sender_id, seq, ciphertext, nonce, client_msg_id, sent_at
           FROM messages WHERE conversation_id = $1 AND seq > $2
           ORDER BY seq ASC LIMIT $3",
    )
    .bind(conversation_id)
    .bind(since_seq)
    .bind(limit.clamp(1, MAX_HISTORY_LIMIT))
    .fetch_all(pool)
    .await
    .map_err(MessagingError::Database)?;
    Ok(rows.into_iter().map(to_message).collect())
}

/// Claim up to `limit` unpublished outbox rows (oldest first) for delivery.
/// Crash-safe: a row claimed but never marked is re-claimed next round
/// (`attempts` grows), so consumers MUST dedup by event id.
pub async fn claim_outbox(
    pool: &sqlx::PgPool,
    limit: i64,
) -> Result<Vec<OutboxEntry>, MessagingError> {
    let rows: Vec<(Uuid, String, serde_json::Value)> = sqlx::query_as(
        r"UPDATE outbox SET attempts = attempts + 1 WHERE id IN (
            SELECT id FROM outbox WHERE published_at IS NULL
            ORDER BY created_at ASC LIMIT $1 FOR UPDATE SKIP LOCKED
          ) RETURNING id, topic, payload",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(MessagingError::Database)?;
    Ok(rows
        .into_iter()
        .map(|row| OutboxEntry {
            id: row.0,
            topic: row.1,
            payload: row.2,
        })
        .collect())
}

/// Stamp claimed rows delivered.
pub async fn mark_published(pool: &sqlx::PgPool, ids: &[Uuid]) -> Result<(), MessagingError> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE outbox SET published_at = now() WHERE id = ANY($1)")
        .bind(ids)
        .execute(pool)
        .await
        .map_err(MessagingError::Database)?;
    Ok(())
}

/// Events visible to `user_id` after `after` (exclusive, `None` = from the
/// start), oldest first. Drives gateway resume; `UUIDv7` ids are the global
/// order. Membership-filtered through participants.
pub async fn events_after(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<OutboxEntry>, MessagingError> {
    let rows: Vec<(Uuid, String, serde_json::Value)> = sqlx::query_as(
        r"SELECT o.id, o.topic, o.payload FROM outbox o
          WHERE (payload->>'conversation_id')::uuid IN (
              SELECT conversation_id FROM conversation_participants WHERE user_id = $1
          )
          AND (CAST($2 AS UUID) IS NULL OR o.id > CAST($2 AS UUID))
          ORDER BY o.id ASC LIMIT $3",
    )
    .bind(user_id)
    .bind(after)
    .bind(limit.clamp(1, MAX_HISTORY_LIMIT))
    .fetch_all(pool)
    .await
    .map_err(MessagingError::Database)?;
    Ok(rows
        .into_iter()
        .map(|row| OutboxEntry {
            id: row.0,
            topic: row.1,
            payload: row.2,
        })
        .collect())
}

/// Background outbox worker: claim → broadcast → mark, every 500 ms.
/// At-least-once by design; `main` aborts it on shutdown and the next boot
/// re-claims anything unmarked.
pub async fn outbox_worker(pool: sqlx::PgPool, hub: broadcast::Sender<OutboxEntry>) {
    loop {
        match claim_outbox(&pool, OUTBOX_BATCH).await {
            Err(err) => {
                tracing::error!("outbox claim failed: {err}");
            }
            Ok(entries) if entries.is_empty() => {}
            Ok(entries) => {
                let ids: Vec<Uuid> = entries.iter().map(|entry| entry.id).collect();
                for entry in &entries {
                    // No receivers yet (or all lagged) is fine — resume covers it.
                    let _ = hub.send(entry.clone());
                }
                if let Err(err) = mark_published(&pool, &ids).await {
                    tracing::error!("outbox mark-published failed: {err}");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Register one user for messaging tests, returning its id.
    async fn member(pool: &sqlx::PgPool, stamp: i64, name: &str) -> Uuid {
        crate::auth::create_user(
            pool,
            &format!("{name}{stamp}"),
            &format!("{name}{stamp}@example.com"),
            name,
            "pw-group-1",
        )
        .await
        .expect("register member")
        .id
    }
    /// Requires a live database; skips honestly without one. DM dedup,
    /// idempotent send, sequencing, history, and the membership wall.
    #[tokio::test]
    async fn dm_send_history_idempotency() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: dm_send_history_idempotency (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = crate::auth::create_user(
            &pool,
            &format!("alice{stamp}"),
            &format!("alice{stamp}@example.com"),
            "Alice",
            "pw-alice-1",
        )
        .await
        .expect("register alice");
        let bob = crate::auth::create_user(
            &pool,
            &format!("bob{stamp}"),
            &format!("bob{stamp}@example.com"),
            "Bob",
            "pw-bob-1",
        )
        .await
        .expect("register bob");
        let mallory = crate::auth::create_user(
            &pool,
            &format!("mallory{stamp}"),
            &format!("mallory{stamp}@example.com"),
            "Mallory",
            "pw-mallory-1",
        )
        .await
        .expect("register mallory");

        // DM creation is idempotent: same pair, same conversation.
        let dm = find_or_create_dm(&pool, alice.id, bob.id)
            .await
            .expect("create dm");
        let dm_again = find_or_create_dm(&pool, bob.id, alice.id)
            .await
            .expect("reopen dm");
        assert_eq!(dm.id, dm_again.id);

        // The first encrypted message is `bro` (opaque to the server).
        let key = Uuid::now_v7();
        let (first, created) = send_message(&pool, alice.id, dm.id, key, b"bro", None)
            .await
            .expect("send bro");
        assert!(created);
        assert_eq!(first.seq, 1);
        assert_eq!(first.ciphertext, b"bro");

        // Retried send returns the original without a new sequence or event.
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbox")
            .fetch_one(&pool)
            .await
            .expect("count outbox");
        let (retry, created) = send_message(&pool, alice.id, dm.id, key, b"bro", None)
            .await
            .expect("retry send");
        assert!(!created);
        assert_eq!(retry.id, first.id);
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbox")
            .fetch_one(&pool)
            .await
            .expect("count outbox");
        assert_eq!(before, after, "retry must not emit an event");

        // Bob reads history; Mallory hits the membership wall either way.
        let history = message_history(&pool, bob.id, dm.id, 0, 50)
            .await
            .expect("bob history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].seq, 1);
        assert!(message_history(&pool, mallory.id, dm.id, 0, 50)
            .await
            .is_err());
        assert!(
            send_message(&pool, mallory.id, dm.id, Uuid::now_v7(), b"hi", None)
                .await
                .is_err()
        );

        // Claim → mark drains the outbox exactly once per event.
        let claimed = claim_outbox(&pool, 10).await.expect("claim");
        assert!(claimed.iter().any(|entry| entry.topic == "message.created"));
        let ids: Vec<Uuid> = claimed.iter().map(|entry| entry.id).collect();
        mark_published(&pool, &ids).await.expect("mark");
        let drained = claim_outbox(&pool, 10).await.expect("reclaim");
        assert!(drained.is_empty());
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Group creation,
    /// fan-out visibility per member, and the outsider wall.
    #[tokio::test]
    async fn group_send_history() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: group_send_history (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let ada = member(&pool, stamp, "grpada").await;
        let bob = member(&pool, stamp, "grpbob").await;
        let cat = member(&pool, stamp, "grpcat").await;
        let out = member(&pool, stamp, "grpout").await;

        let group = create_conversation(&pool, ada, "group", &[bob, cat])
            .await
            .expect("create group");
        assert_eq!(group.kind, "group");
        assert_eq!(group.members.len(), 3);

        let key = Uuid::now_v7();
        let (first, created) =
            send_message(&pool, bob, group.id, key, b"group-bro", Some(b"n0nce"))
                .await
                .expect("group send");
        assert!(created);
        assert_eq!(first.seq, 1);

        // Every member reads it with the nonce intact; the outsider cannot.
        for member_id in [ada, bob, cat] {
            let history = message_history(&pool, member_id, group.id, 0, 50)
                .await
                .expect("member history");
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].ciphertext, b"group-bro");
            assert_eq!(history[0].nonce, Some(b"n0nce".to_vec()));
        }
        assert!(message_history(&pool, out, group.id, 0, 50).await.is_err());

        // Pagination: since_seq filters.
        let page = message_history(&pool, ada, group.id, 1, 50)
            .await
            .expect("page");
        assert!(page.is_empty());
        pool.close().await;
    }
}
