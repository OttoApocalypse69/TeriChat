//! Versioned realtime gateway (`/v1/gateway`).
//!
//! Wire protocol (JSON text frames, client → server):
//! - `{"op":"identify","resume_after":"<event-uuid>"|null}` — required first.
//!   The server replies `ready`, replays missed events, then streams live ones.
//! - `{"op":"heartbeat","seq":n}` — echoed as `heartbeat_ack`.
//!
//! Server → client: `{"op":"ready",...}`, `{"op":"event","event":{...}}`,
//! `{"op":"heartbeat_ack","seq":n}`, `{"op":"error","code":...}`.
//!
//! Auth rides the `?token=` query parameter (bearer). Authorization is
//! evaluated live per event (not snapshotted): a kick/ban mid-connection
//! stops delivery without a reconnect, and joining mid-connection starts it.
//! Delivery is at-least-once — clients dedup by `event_id` and resume with
//! the last one they processed. UUIDs are allocation-ordered, not commit-
//! ordered: reconnecting clients must also reconcile each conversation's
//! sequence history, including quiet conversations. Live frames are deduped
//! by exact id only (bounded cache); lag triggers a full retained-history scan.
//! Identify, database steps and socket writes have five-second budgets. During
//! replay the pump still services inbound close/heartbeat frames between events.
//! Post-identify heartbeats have no liveness timeout (a documented gap).

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::Response,
};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::auth;
use crate::messaging::{self, OutboxEntry};
use crate::{errors::AppError, state::AppState, workspaces};

/// `GET /v1/gateway?token=...` query.
#[derive(Debug, Deserialize)]
pub struct GatewayParams {
    /// Opaque bearer token from login.
    pub token: Option<String>,
}

/// Client → server frame.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ClientFrame {
    /// Authenticate the connection and start the stream.
    Identify {
        /// Last processed event id, for resume. `None` = from the start.
        resume_after: Option<Uuid>,
    },
    /// Liveness ping, echoed back.
    Heartbeat {
        /// Client sequence, echoed verbatim.
        seq: i64,
    },
}

/// Server → client frame.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ServerFrame {
    /// Connection authenticated and streaming.
    Ready {
        /// Connection session id (observability only).
        session_id: Uuid,
        /// Authenticated account.
        user_id: Uuid,
    },
    /// Heartbeat echo.
    HeartbeatAck {
        /// Client sequence echoed verbatim.
        seq: i64,
    },
    /// Protocol error. The connection stays open.
    Error {
        /// Machine-readable code.
        code: &'static str,
    },
}

async fn send_event(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    entry: &OutboxEntry,
) -> bool {
    let frame = serde_json::json!({
        "op": "event",
        "event": entry,
    });
    matches!(
        tokio::time::timeout(
            IO_TIMEOUT,
            sink.send(Message::Text(frame.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    )
}

/// Upgrade to the gateway WebSocket after bearer auth.
pub async fn gateway_handler(
    State(state): State<AppState>,
    Query(params): Query<GatewayParams>,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let token = params
        .token
        .filter(|t| !t.is_empty())
        .ok_or(AppError::Unauthorized)?;
    let session = auth::authenticate(pool, &token)
        .await
        .map_err(|_| AppError::Unauthorized)?;
    Ok(ws.on_upgrade(move |socket| connection(socket, state, session)))
}

/// Bound identification independently of client heartbeat traffic.
async fn identify(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<Option<Uuid>> {
    // A fixed deadline, not a sliding one: pre-identify heartbeats cannot
    // keep an otherwise idle authenticated socket alive indefinitely.
    let identify_deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    loop {
        let incoming = tokio::select! {
            incoming = stream.next() => incoming,
            () = tokio::time::sleep_until(identify_deadline) => return None,
        };
        match incoming {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientFrame>(&text) {
                Ok(ClientFrame::Identify { resume_after }) => return Some(resume_after),
                Ok(ClientFrame::Heartbeat { seq }) => {
                    if !send_frame(sink, &ServerFrame::HeartbeatAck { seq }).await {
                        return None;
                    }
                }
                Err(_) => {
                    if !send_frame(sink, &ServerFrame::Error { code: "bad_frame" }).await {
                        return None;
                    }
                }
            },
            Some(Ok(Message::Close(_)) | Err(_)) | None => return None,
            // Binary/ping/pong/pong frames are ignored pre-identify.
            _ => {}
        }
    }
}

/// Per-connection pump: identify → replay → live.
async fn connection(socket: WebSocket, state: AppState, session: auth::AuthSession) {
    let (mut sink, mut stream) = socket.split();
    let Some(pool) = &state.pool else {
        send_frame(
            &mut sink,
            &ServerFrame::Error {
                code: "no_database",
            },
        )
        .await;
        return;
    };
    let pool = pool.clone();
    let mut hub_rx = state.hub.subscribe();

    let Some(resume_after) = identify(&mut sink, &mut stream).await else {
        return;
    };

    // Subscribe before replay; UUIDs order allocation, never commit visibility.
    let Some(mut replay) = Replay::start(&pool, session.user_id, resume_after).await else {
        return;
    };
    let mut seen = std::collections::VecDeque::new();
    if !send_frame(
        &mut sink,
        &ServerFrame::Ready {
            session_id: Uuid::now_v7(),
            user_id: session.user_id,
        },
    )
    .await
    {
        return;
    }
    loop {
        let entry = tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let frame = match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(ClientFrame::Heartbeat { seq }) => ServerFrame::HeartbeatAck { seq },
                            Ok(ClientFrame::Identify { .. }) => ServerFrame::Error { code: "already_identified" },
                            Err(_) => ServerFrame::Error { code: "bad_frame" },
                        };
                        if !send_frame(&mut sink, &frame).await { return; }
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                    _ => {}
                }
                continue;
            }
            page = tokio::time::timeout(IO_TIMEOUT, replay.next(&pool, session.user_id)), if !replay.done => {
                match page {
                    Ok(Ok(Some(entry))) => entry,
                    Ok(Ok(None)) => continue,
                    _ => return, // Never silently hand off an incomplete replay.
                }
            }
            live = hub_rx.recv(), if replay.done => {
                match live {
                    Ok(entry) => entry,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // UUID cursors cannot recover late commits below a
                        // watermark. On lag rescan retained history instead.
                        let Some(scan) = Replay::start(&pool, session.user_id, None).await else { return; };
                        replay = scan;
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        };
        if seen.contains(&entry.id) {
            continue;
        }
        let Ok(visible) =
            tokio::time::timeout(IO_TIMEOUT, event_visible(&pool, session.user_id, &entry)).await
        else {
            return;
        };
        if visible {
            if !send_event(&mut sink, &entry).await {
                return;
            }
            if seen.len() == 1024 {
                seen.pop_front();
            }
            seen.push_back(entry.id);
        }
    }
}

const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// One bounded page at a time, with a finite allocation ceiling. The scan
/// cursor advances even for invisible events. Buffered rows are cancellation
/// safe: no await occurs between popping a row and returning it to the pump.
struct Replay {
    cursor: Option<Uuid>,
    ceiling: Option<Uuid>,
    pending: std::collections::VecDeque<OutboxEntry>,
    done: bool,
}

impl Replay {
    async fn start(pool: &sqlx::PgPool, user: Uuid, cursor: Option<Uuid>) -> Option<Self> {
        if !matches!(
            tokio::time::timeout(IO_TIMEOUT, workspaces::ensure_all_participation(pool, user))
                .await,
            Ok(Ok(()))
        ) {
            return None;
        }

        let Ok(Ok(ceiling)) = tokio::time::timeout(
            IO_TIMEOUT,
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM outbox ORDER BY id DESC LIMIT 1")
                .fetch_optional(pool),
        )
        .await
        else {
            return None;
        };
        Some(Self {
            cursor,
            ceiling,
            pending: std::collections::VecDeque::new(),
            done: ceiling.is_none(),
        })
    }

    async fn next(
        &mut self,
        pool: &sqlx::PgPool,
        user: Uuid,
    ) -> Result<Option<OutboxEntry>, messaging::MessagingError> {
        if self.pending.is_empty() {
            if self.cursor >= self.ceiling {
                self.done = true;
                return Ok(None);
            }
            let page = messaging::events_after(pool, user, self.cursor, 100).await?;
            for entry in page {
                if self.ceiling.is_some_and(|ceiling| entry.id > ceiling) {
                    break;
                }
                self.cursor = Some(entry.id);
                self.pending.push_back(entry);
            }
            if self.pending.is_empty() {
                self.done = true;
            }
        }
        Ok(self.pending.pop_front())
    }
}

/// Whether `user_id` may currently read this event. Channel events need
/// workspace membership (reads are member-wide — guests included; `SEND` is
/// the write gate and is not consulted here); anything else needs
/// conversation participation. Evaluated live per event so kicks, bans, and
/// mutes take effect mid-connection. Unknown-shape events fail closed.
async fn event_visible(pool: &sqlx::PgPool, user_id: Uuid, entry: &OutboxEntry) -> bool {
    let Some(conversation) = entry
        .payload
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .and_then(|raw| raw.parse::<Uuid>().ok())
    else {
        return false;
    };
    match workspaces::channel_by_conversation(pool, conversation).await {
        Ok(Some(channel)) => workspaces::get_workspace(pool, user_id, channel.workspace_id)
            .await
            .is_ok(),
        Ok(None) => messaging::is_member(pool, conversation, user_id)
            .await
            .unwrap_or(false),
        Err(_) => false,
    }
}

async fn send_frame(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: &ServerFrame,
) -> bool {
    let text = serde_json::to_string(frame).unwrap_or_else(|_| r#"{"op":"error"}"#.to_owned());
    matches!(
        tokio::time::timeout(IO_TIMEOUT, sink.send(Message::Text(text.into()))).await,
        Ok(Ok(()))
    )
}
