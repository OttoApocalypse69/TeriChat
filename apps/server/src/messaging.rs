//! Milestone C messaging: conversations, opaque envelopes, transactional outbox.
//!
//! Privacy shape: the server routes ciphertext it cannot read. Sends and
//! history are membership-checked; a non-member gets [`MessagingError::NotMember`]
//! whether the conversation exists or not (no existence oracle). Retried sends
//! with the same `client_msg_id` return the original row — no duplicates.
//! Outbox delivery is at-least-once: consumers must dedup by event id.

use chrono::{DateTime, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
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

/// Conversation list entry for `GET /v1/conversations`.
///
/// Everything here is scoped to conversations the caller already belongs
/// to: the peer identity is resolved server-side from the membership rows
/// (no handle oracle — a stranger's handle can never be probed through
/// this endpoint), plus the last message position for previews/sync.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    /// Conversation id (`UUIDv7`).
    pub id: Uuid,
    /// `dm`, `group`, or `channel`.
    pub kind: String,
    /// Member account ids.
    pub members: Vec<Uuid>,
    /// For a two-member `dm`: the other member's handle. `None` otherwise.
    pub peer_handle: Option<String>,
    /// For a two-member `dm`: the other member's display name. `None` otherwise.
    pub peer_display_name: Option<String>,
    /// Highest `seq` sent in this conversation, if any message exists.
    pub last_seq: Option<i64>,
    /// `sent_at` of that last message, if any message exists.
    pub last_sent_at: Option<DateTime<Utc>>,
    /// The caller's own read position (highest seq they have read). Private:
    /// never another member's marker.
    pub last_read_seq: i64,
    /// Handle and display name of every member of a `dm` or `group`, so
    /// clients can label senders. Empty for `channel`: workspace membership
    /// has its own gated, paged directory.
    pub member_profiles: Vec<MemberProfile>,
}

/// Public identity of a conversation member, as seen by co-members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberProfile {
    /// Member account id.
    pub user_id: Uuid,
    /// Unique handle.
    pub handle: String,
    /// Display name.
    pub display_name: String,
}

/// One row of the conversation list query: id, kind, last seq, last time,
/// and the caller's read marker.
type ConversationListRow = (Uuid, String, Option<i64>, Option<DateTime<Utc>>, i64);

/// List the caller's conversations (dm/group/channel rows where the caller
/// is a participant), newest activity first.
///
/// For two-member `dm`s the peer handle + display name are resolved from
/// the `users` table — the caller is a member, so this reveals nothing new
/// (no oracle). Groups and channels carry `peer_* = None`; clients label
/// those from their own membership/workspace state.
///
/// # Errors
///
/// Returns [`MessagingError::Database`] when the conversation query fails.
pub async fn list_conversations(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<ConversationSummary>, MessagingError> {
    // One row per member conversation with its last message position.
    // Correlated subqueries keep this a single round-trip; Alpha-sized lists
    // make this cheaper than a join + dedup in code.
    let rows: Vec<ConversationListRow> = sqlx::query_as(
        r"SELECT c.id, c.kind,
            (SELECT MAX(m.seq) FROM messages m WHERE m.conversation_id = c.id),
            (SELECT MAX(m.sent_at) FROM messages m WHERE m.conversation_id = c.id),
            p.last_read_seq
          FROM conversations c
          JOIN conversation_participants p ON p.conversation_id = c.id
          WHERE p.user_id = $1
          ORDER BY 4 DESC NULLS LAST, c.id ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(MessagingError::Database)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<Uuid> = rows.iter().map(|row| row.0).collect();
    // All member ids for exactly these conversations (still caller-scoped:
    // every row belongs to a conversation the caller is in).
    let member_rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT conversation_id, user_id FROM conversation_participants
         WHERE conversation_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(MessagingError::Database)?;
    let mut members_by_conv: std::collections::HashMap<Uuid, Vec<Uuid>> =
        std::collections::HashMap::new();
    for (conv_id, member_id) in member_rows {
        members_by_conv.entry(conv_id).or_default().push(member_id);
    }

    // Profiles for every member of the caller's dm/group rows (channels are
    // excluded; their membership is the workspace directory's concern).
    let has_profiles = |kind: &str| kind == "dm" || kind == "group";
    let mut profile_ids: Vec<Uuid> = rows
        .iter()
        .filter(|row| has_profiles(&row.1))
        .filter_map(|row| members_by_conv.get(&row.0))
        .flatten()
        .copied()
        .collect();
    profile_ids.sort_unstable();
    profile_ids.dedup();
    let profile_rows: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, handle, display_name FROM users WHERE id = ANY($1)")
            .bind(&profile_ids)
            .fetch_all(pool)
            .await
            .map_err(MessagingError::Database)?;
    let profiles: std::collections::HashMap<Uuid, (String, String)> = profile_rows
        .into_iter()
        .map(|row| (row.0, (row.1, row.2)))
        .collect();

    Ok(rows
        .into_iter()
        .map(|row| {
            let members = members_by_conv.remove(&row.0).unwrap_or_default();
            // Peer: the non-caller member of a two-member DM only.
            let peer = if row.1 == "dm" && members.len() == 2 {
                members
                    .iter()
                    .find(|id| **id != user_id)
                    .and_then(|id| profiles.get(id))
            } else {
                None
            };
            let mut member_profiles: Vec<MemberProfile> = if has_profiles(&row.1) {
                members
                    .iter()
                    .filter_map(|id| {
                        profiles
                            .get(id)
                            .map(|(handle, display_name)| MemberProfile {
                                user_id: *id,
                                handle: handle.clone(),
                                display_name: display_name.clone(),
                            })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            member_profiles.sort_by(|a, b| a.handle.cmp(&b.handle));
            ConversationSummary {
                id: row.0,
                kind: row.1,
                members,
                peer_handle: peer.map(|p| p.0.clone()),
                peer_display_name: peer.map(|p| p.1.clone()),
                last_seq: row.2,
                last_sent_at: row.3,
                last_read_seq: row.4,
                member_profiles,
            }
        })
        .collect())
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

/// Create a conversation of `kind` (`dm`/`group`/`channel`) with `members`.
/// The creator is added when missing. Empty membership is rejected.
/// `channel` rows are created by the workspaces module with a linked channel;
/// see `channels.conversation_id`.
///
/// # Errors
///
/// Returns [`MessagingError::Database`] when the insert fails.
pub async fn create_conversation(
    pool: &sqlx::PgPool,
    creator: Uuid,
    kind: &str,
    members: &[Uuid],
) -> Result<Conversation, MessagingError> {
    let mut tx = pool.begin().await.map_err(MessagingError::Database)?;
    let conversation = insert_conversation(&mut tx, creator, kind, members).await?;
    tx.commit().await.map_err(MessagingError::Database)?;
    Ok(conversation)
}

/// Shared creation path: every row uses the caller's one transaction/connection.
/// A failed insert or cancelled future drops the transaction without publishing
/// a conversation with incomplete membership.
async fn insert_conversation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    creator: Uuid,
    kind: &str,
    members: &[Uuid],
) -> Result<Conversation, MessagingError> {
    if kind != "dm" && kind != "group" && kind != "channel" {
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
        .execute(&mut **tx)
        .await
        .map_err(MessagingError::Database)?;
    for member in &all {
        sqlx::query(
            "INSERT INTO conversation_participants (conversation_id, user_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(member)
        .execute(&mut **tx)
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
/// creation idempotent for distinct users: repeated calls return the same
/// conversation. The existing self-DM behavior (new singleton per call) remains.
///
/// # Errors
///
/// Returns [`MessagingError::Database`] when the lookup or creation fails.
pub async fn find_or_create_dm(
    pool: &sqlx::PgPool,
    a: Uuid,
    b: Uuid,
) -> Result<Conversation, MessagingError> {
    let mut tx = pool.begin().await.map_err(MessagingError::Database)?;
    // The lookup must take a fresh snapshot AFTER the previous lock holder
    // commits, including when a deployment changes its session default.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await
        .map_err(MessagingError::Database)?;
    // Canonical UUID bytes make reversed pairs share one transaction-scoped
    // lock across processes. A hash collision only serializes unrelated pairs;
    // identity is still checked using the full UUIDs below. This is a lock key,
    // not a cryptographic protocol or a persistent identity.
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha256::new();
    hasher.update(b"terichat/dm-creation-lock/v1");
    hasher.update(low.as_bytes());
    hasher.update(high.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0_u8; 8];
    key.copy_from_slice(&digest[..8]);
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(i64::from_be_bytes(key))
        .execute(&mut *tx)
        .await
        .map_err(MessagingError::Database)?;
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
    .fetch_optional(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;

    let conversation = if let Some(id) = existing {
        let members: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM conversation_participants WHERE conversation_id = $1",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await
        .map_err(MessagingError::Database)?;
        Conversation {
            id,
            kind: "dm".to_owned(),
            members,
        }
    } else {
        // Never reacquire from the pool while holding a transaction: five
        // simultaneous requests must also complete with a five-connection pool.
        insert_conversation(&mut tx, a, "dm", &[b]).await?
    };
    tx.commit().await.map_err(MessagingError::Database)?;
    Ok(conversation)
}

/// Authorize a typing signal and capture its audience under the typist's
/// participant row lock, handing both to `publish` before the lock is
/// released. A concurrent kick's DELETE waits for this (as it does for a
/// send), so a signal is either published before a removal or refused after
/// it: never authorized before a kick and captured after it. `publish` gets
/// every participant plus the conversation's highest message seq.
///
/// # Errors
///
/// Returns [`MessagingError::NotMember`] when `user_id` is not a participant
/// and [`MessagingError::Database`] on database failure.
pub async fn publish_typing(
    pool: &sqlx::PgPool,
    conversation_id: Uuid,
    user_id: Uuid,
    publish: impl FnOnce(Vec<Uuid>, i64),
) -> Result<(), MessagingError> {
    let mut tx = pool.begin().await.map_err(MessagingError::Database)?;
    let member: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM conversation_participants
         WHERE conversation_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;
    if member.is_none() {
        return Err(MessagingError::NotMember);
    }
    let recipients: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM conversation_participants WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;
    let last_seq: i64 = sqlx::query_scalar("SELECT next_seq - 1 FROM conversations WHERE id = $1")
        .bind(conversation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(MessagingError::Database)?;
    publish(recipients, last_seq);
    tx.commit().await.map_err(MessagingError::Database)
}

/// Membership check. `false` covers both non-members and missing rows.
///
/// # Errors
///
/// Returns [`MessagingError::Database`] when the lookup fails.
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
///
/// # Errors
///
/// Returns [`MessagingError::EmptyCiphertext`] for empty envelopes,
/// [`MessagingError::CiphertextTooLarge`] for oversized envelopes,
/// [`MessagingError::NotMember`] when the sender may not write here, or
/// [`MessagingError::Database`] when the write fails.
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
    // FOR UPDATE pins the participant row for the life of this tx: a
    // concurrent kick's DELETE blocks here instead of slipping between this
    // check and the send below, so send-then-kick and kick-then-send are the
    // only two possible orders. The lock is consistent (participant row first,
    // conversation row second) everywhere both are touched, so no deadlock.
    let member: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM conversation_participants
         WHERE conversation_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(conversation_id)
    .bind(sender_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;
    if member.is_none() {
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
    // A vanished conversation here means a concurrent channel delete slipped
    // in after the membership check: report absence, not a 500.
    let seq: i64 = sqlx::query_scalar(
        "UPDATE conversations SET next_seq = next_seq + 1 WHERE id = $1 RETURNING next_seq - 1",
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| match err {
        sqlx::Error::RowNotFound => MessagingError::NotMember,
        other => MessagingError::Database(other),
    })?;

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
    // Your own message is never unread for you, but only when you were
    // already caught up: if someone else's message took the previous seq and
    // you have not marked it read, stepping over it would hide it. The
    // participant row is locked above, so this cannot race a marker update.
    sqlx::query(
        "UPDATE conversation_participants SET last_read_seq = $3
         WHERE conversation_id = $1 AND user_id = $2 AND last_read_seq = $3 - 1",
    )
    .bind(conversation_id)
    .bind(sender_id)
    .bind(seq)
    .execute(&mut *tx)
    .await
    .map_err(MessagingError::Database)?;
    tx.commit().await.map_err(MessagingError::Database)?;
    Ok((message, true))
}

/// Advance the caller's private read marker to `seq`, returning the stored
/// marker. Markers only move forward and never past the last assigned
/// sequence, so a stale or oversized request cannot rewind or skip ahead.
///
/// # Errors
///
/// [`MessagingError::NotMember`] when the caller is not a participant;
/// [`MessagingError::Database`] on query failure.
pub async fn mark_read(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
    seq: i64,
) -> Result<i64, MessagingError> {
    let marker: Option<i64> = sqlx::query_scalar(
        r"UPDATE conversation_participants p
             SET last_read_seq = GREATEST(p.last_read_seq, LEAST($3, c.next_seq - 1)),
                 last_read_at = now()
            FROM conversations c
           WHERE c.id = p.conversation_id AND p.conversation_id = $1 AND p.user_id = $2
       RETURNING p.last_read_seq",
    )
    .bind(conversation_id)
    .bind(user_id)
    .bind(seq.max(0))
    .fetch_optional(pool)
    .await
    .map_err(MessagingError::Database)?;
    marker.ok_or(MessagingError::NotMember)
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
///
/// # Errors
///
/// Returns [`MessagingError::NotMember`] for non-members, or
/// [`MessagingError::Database`] when the query fails.
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
///
/// # Errors
///
/// Returns [`MessagingError::Database`] when the claim query fails.
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
///
/// # Errors
///
/// Returns [`MessagingError::Database`] when the update fails.
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
/// start), allocation-ordered. This is a scan cursor, NOT a commit watermark:
/// late commits below `after` require client history reconciliation on resume.
/// The gateway subscribes before scanning, never UUID-filters live events,
/// and rescans retained history on broadcast lag. Membership prefiltered via
/// participants; the gateway applies authoritative visibility before sending.
/// Compare text rather than casting untrusted payload fields to UUID: malformed
/// unrelated events must not abort every user's replay.
///
/// # Errors
///
/// Returns [`MessagingError::Database`] when the scan query fails.
pub async fn events_after(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<OutboxEntry>, MessagingError> {
    let rows: Vec<(Uuid, String, serde_json::Value)> = sqlx::query_as(
        r"SELECT o.id, o.topic, o.payload FROM outbox o
          WHERE payload->>'conversation_id' IN (
              SELECT conversation_id::text FROM conversation_participants WHERE user_id = $1
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
        let before: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM outbox WHERE payload->>'conversation_id' = $1",
        )
        .bind(dm.id.to_string())
        .fetch_one(&pool)
        .await
        .expect("count outbox");
        let (retry, created) = send_message(&pool, alice.id, dm.id, key, b"bro", None)
            .await
            .expect("retry send");
        assert!(!created);
        assert_eq!(retry.id, first.id);
        let after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM outbox WHERE payload->>'conversation_id' = $1",
        )
        .bind(dm.id.to_string())
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

    /// Requires a live database; skips honestly without one. The
    /// conversation list returns the caller's conversations with the DM
    /// peer resolved server-side plus the last message position — and a
    /// stranger sees none of it (no oracle).
    #[tokio::test]
    async fn list_conversations_dm_peer_and_last_message() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: list_conversations_dm_peer_and_last_message (DATABASE_URL unset)");
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
            &format!("lstalice{stamp}"),
            &format!("lstalice{stamp}@example.com"),
            "List Alice",
            "pw-list-alice-1",
        )
        .await
        .expect("register alice");
        let bob = crate::auth::create_user(
            &pool,
            &format!("lstbob{stamp}"),
            &format!("lstbob{stamp}@example.com"),
            "List Bob",
            "pw-list-bob-1",
        )
        .await
        .expect("register bob");
        let mallory = crate::auth::create_user(
            &pool,
            &format!("lstmallory{stamp}"),
            &format!("lstmallory{stamp}@example.com"),
            "List Mallory",
            "pw-list-mallory-1",
        )
        .await
        .expect("register mallory");

        // Empty list before any conversation exists.
        let empty = list_conversations(&pool, alice.id)
            .await
            .expect("empty list");
        assert!(empty.is_empty());

        let dm = find_or_create_dm(&pool, alice.id, bob.id)
            .await
            .expect("create dm");
        let group = create_conversation(&pool, alice.id, "group", &[bob.id])
            .await
            .expect("create group");

        // One message in the DM only: the group stays quiet (NULL position).
        send_message(&pool, alice.id, dm.id, Uuid::now_v7(), b"hey", None)
            .await
            .expect("dm send");

        let listed = list_conversations(&pool, alice.id)
            .await
            .expect("list alice");
        assert_eq!(listed.len(), 2, "alice sees dm + group");

        let dm_row = listed.iter().find(|row| row.id == dm.id).expect("dm row");
        assert_eq!(dm_row.kind, "dm");
        assert_eq!(dm_row.members.len(), 2);
        assert_eq!(dm_row.peer_handle.as_deref(), Some(bob.handle.as_str()));
        assert_eq!(
            dm_row.peer_display_name.as_deref(),
            Some(bob.display_name.as_str())
        );
        assert_eq!(dm_row.last_seq, Some(1));
        assert!(dm_row.last_sent_at.is_some());

        let group_row = listed
            .iter()
            .find(|row| row.id == group.id)
            .expect("group row");
        assert_eq!(group_row.kind, "group");
        assert_eq!(group_row.peer_handle, None);
        assert_eq!(group_row.peer_display_name, None);
        assert_eq!(group_row.last_seq, None);
        assert_eq!(group_row.last_sent_at, None);

        // Bob sees the same DM with Alice as the peer.
        let bob_listed = list_conversations(&pool, bob.id).await.expect("list bob");
        let bob_dm = bob_listed
            .iter()
            .find(|row| row.id == dm.id)
            .expect("bob dm row");
        assert_eq!(bob_dm.peer_handle.as_deref(), Some(alice.handle.as_str()));

        // Mallory is in neither conversation: sees nothing (no oracle).
        let mallory_listed = list_conversations(&pool, mallory.id)
            .await
            .expect("list mallory");
        assert!(!mallory_listed.iter().any(|row| row.id == dm.id));
        assert!(!mallory_listed.iter().any(|row| row.id == group.id));
        pool.close().await;
    }

    #[tokio::test]
    async fn list_conversations_member_profiles_scope() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: list_conversations_member_profiles_scope (DATABASE_URL unset)");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test database");
        crate::MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut users = Vec::new();
        for (name, display) in [
            ("mpada", "Profile Ada"),
            ("mpbob", "Profile Bob"),
            ("mpcat", "Profile Cat"),
            ("mpdan", "Profile Dan"),
        ] {
            users.push(
                crate::auth::create_user(
                    &pool,
                    &format!("{name}{stamp}"),
                    &format!("{name}{stamp}@example.com"),
                    display,
                    "pw-profile-1",
                )
                .await
                .expect("register"),
            );
        }
        let (ada, bob, cat, dan) = (&users[0], &users[1], &users[2], &users[3]);

        let group = create_conversation(&pool, ada.id, "group", &[bob.id, cat.id])
            .await
            .expect("create group");
        let dm = find_or_create_dm(&pool, ada.id, bob.id)
            .await
            .expect("create dm");
        // Channel participants never leak through this list.
        let channel = create_conversation(&pool, ada.id, "channel", &[dan.id])
            .await
            .expect("create channel conversation");

        let listed = list_conversations(&pool, ada.id).await.expect("list ada");
        let row = |id: Uuid| listed.iter().find(|row| row.id == id).expect("row");

        let group_profiles = &row(group.id).member_profiles;
        assert_eq!(
            group_profiles
                .iter()
                .map(|p| p.user_id)
                .collect::<std::collections::HashSet<_>>(),
            [ada.id, bob.id, cat.id].into_iter().collect(),
            "every group member, including the caller"
        );
        let cat_profile = group_profiles
            .iter()
            .find(|p| p.user_id == cat.id)
            .expect("cat");
        assert_eq!(cat_profile.handle, cat.handle);
        assert_eq!(cat_profile.display_name, cat.display_name);
        assert!(group_profiles
            .windows(2)
            .all(|pair| pair[0].handle <= pair[1].handle));

        assert_eq!(row(dm.id).member_profiles.len(), 2);
        assert!(row(channel.id).member_profiles.is_empty());
        assert!(!listed
            .iter()
            .flat_map(|row| &row.member_profiles)
            .any(|p| p.user_id == dan.id));

        // A non-member sees neither the group nor its roster.
        let dan_listed = list_conversations(&pool, dan.id).await.expect("list dan");
        assert!(!dan_listed.iter().any(|row| row.id == group.id));
        pool.close().await;
    }
}
