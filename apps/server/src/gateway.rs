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
//! Auth rides the `?token=` query parameter (bearer). Membership is snapshotted
//! at `identify`: joining a conversation mid-connection needs a reconnect.
//! Delivery is at-least-once — clients dedup by `event_id` and resume with
//! the last one they processed. Heartbeats are currently echoed without
//! server-side timeout enforcement (documented gap, not a silent guarantee).

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
use crate::{AppError, AppState};

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
) {
    let frame = serde_json::json!({
        "op": "event",
        "event": entry,
    });
    if sink
        .send(Message::Text(frame.to_string().into()))
        .await
        .is_err()
    {
        // Receiver gone; the pump exits on its next poll.
    }
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

    // First frame must be `identify`.
    let resume_after = loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientFrame>(&text) {
                Ok(ClientFrame::Identify { resume_after }) => break resume_after,
                Ok(ClientFrame::Heartbeat { seq }) => {
                    send_frame(&mut sink, &ServerFrame::HeartbeatAck { seq }).await;
                }
                Err(_) => {
                    send_frame(&mut sink, &ServerFrame::Error { code: "bad_frame" }).await;
                }
            },
            Some(Ok(Message::Close(_))) | None => return,
            // Binary/ping/pong/pong frames are ignored pre-identify.
            _ => {}
        }
    };

    // Snapshot membership, then replay-then-subscribe without gaps: replay
    // covers everything after `resume_after`, and the live loop dedups
    // anything already sent by tracking the last delivered event id.
    let members: Vec<Uuid> = sqlx::query_scalar(
        "SELECT conversation_id FROM conversation_participants WHERE user_id = $1",
    )
    .bind(session.user_id)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    let connection_id = Uuid::now_v7();
    send_frame(
        &mut sink,
        &ServerFrame::Ready {
            session_id: connection_id,
            user_id: session.user_id,
        },
    )
    .await;

    let mut last_sent = resume_after;
    if let Ok(missed) = messaging::events_after(&pool, session.user_id, resume_after, 100).await {
        for entry in missed {
            send_event(&mut sink, &entry).await;
            last_sent = Some(entry.id);
        }
    }

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(ClientFrame::Heartbeat { seq }) => {
                                send_frame(&mut sink, &ServerFrame::HeartbeatAck { seq }).await;
                            }
                            Ok(ClientFrame::Identify { .. }) => {
                                send_frame(&mut sink, &ServerFrame::Error { code: "already_identified" }).await;
                            }
                            Err(_) => {
                                send_frame(&mut sink, &ServerFrame::Error { code: "bad_frame" }).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    _ => {}
                }
            }
            live = hub_rx.recv() => {
                match live {
                    Ok(entry) => {
                        if last_sent.is_some_and(|last| entry.id <= last) {
                            continue; // Already replayed; at-least-once dedup.
                        }
                        if event_visible(&entry, &members) {
                            send_event(&mut sink, &entry).await;
                            last_sent = Some(entry.id);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // We dropped events: re-anchor from the last one sent.
                        if let Ok(missed) = messaging::events_after(&pool, session.user_id, last_sent, 100).await {
                            for entry in missed {
                                send_event(&mut sink, &entry).await;
                                last_sent = Some(entry.id);
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }
}

/// This connection cares about the event when its conversation is in the
/// identify-time membership snapshot.
fn event_visible(entry: &OutboxEntry, members: &[Uuid]) -> bool {
    entry
        .payload
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .and_then(|raw| raw.parse::<Uuid>().ok())
        .is_some_and(|conversation| members.contains(&conversation))
}

async fn send_frame(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: &ServerFrame,
) {
    let text = serde_json::to_string(frame).unwrap_or_else(|_| r#"{"op":"error"}"#.to_owned());
    let _ = sink.send(Message::Text(text.into())).await;
}
