//! Versioned realtime gateway (`/v1/gateway`).
//!
//! Wire protocol (JSON text frames, client → server):
//! - `{"op":"identify","resume_after":"<event-uuid>"|null}` — required first.
//!   The server replies `ready`, replays missed events, then streams live ones.
//! - `{"op":"heartbeat","seq":n}` — echoed as `heartbeat_ack`.
//!
//! Server → client: `{"op":"ready",...}`, `{"op":"event","event":{...}}`,
//! `{"op":"heartbeat_ack","seq":n}`, `{"op":"error","code":...}`, and the
//! ephemeral `{"op":"typing","conversation_id":...,"user_id":...}`. Typing
//! frames carry no event id, are never replayed and never affect resume.
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
        Extension, Query, State,
    },
    response::Response,
};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::auth;
use crate::messaging::{self, OutboxEntry};
use crate::typing::{TypingBus, TypingSignal};
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
    /// Someone else is typing in a conversation this account can see.
    Typing {
        /// Conversation being typed in.
        conversation_id: Uuid,
        /// Account that is typing.
        user_id: Uuid,
        /// Highest message seq when the signal was published. A client drops
        /// the signal if it has already loaded a later message from this
        /// typist (the message and the signal travel independently).
        last_seq: i64,
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
///
/// # Errors
///
/// Returns [`AppError::NoDatabase`] without a pool,
/// [`AppError::Unauthorized`] for missing/invalid tokens, or the domain
/// error when authentication fails.
pub async fn gateway_handler(
    State(state): State<AppState>,
    Extension(typing): Extension<TypingBus>,
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
    Ok(ws.on_upgrade(move |socket| connection(socket, state, typing, session)))
}

/// Bound identification independently of client heartbeat traffic.
async fn identify(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    pool: &sqlx::PgPool,
    session: &auth::AuthSession,
) -> Option<Option<Uuid>> {
    // A fixed deadline, not a sliding one: pre-identify heartbeats cannot
    // keep an otherwise idle authenticated socket alive indefinitely.
    let identify_deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    loop {
        let incoming = tokio::select! {
            incoming = stream.next() => incoming,
            () = tokio::time::sleep_until(identify_deadline) => return None,
        };
        if !session_live(pool, session).await {
            return None;
        }
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
async fn connection(
    socket: WebSocket,
    state: AppState,
    typing_bus: TypingBus,
    session: auth::AuthSession,
) {
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

    let Some(resume_after) = identify(&mut sink, &mut stream, &pool, &session).await else {
        return;
    };

    // Subscribe before replay; UUIDs order allocation, never commit visibility.
    let Some(mut replay) = Replay::start(&pool, session.user_id, resume_after).await else {
        return;
    };
    let mut seen = std::collections::VecDeque::new();
    if !session_live(&pool, &session).await {
        return;
    }
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
    let mut validity_tick = tokio::time::interval(std::time::Duration::from_secs(1));
    validity_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Subscribed only once streaming: typing from before Ready is stale.
    let mut typing = Some(typing_bus.subscribe());
    loop {
        let entry = match next_delivery(
            &mut sink,
            &mut stream,
            &pool,
            &session,
            &mut replay,
            &mut hub_rx,
            &mut typing,
            &mut validity_tick,
        )
        .await
        {
            Some(Delivery::Event(entry)) => entry,
            Some(Delivery::ReplayEnd) => continue,
            Some(Delivery::Lagged) => {
                let Some(scan) = Replay::start(&pool, session.user_id, None).await else {
                    return;
                };
                replay = scan;
                continue;
            }
            None => return,
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
            if !session_live(&pool, &session).await {
                return;
            }
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

enum Delivery {
    Event(OutboxEntry),
    ReplayEnd,
    Lagged,
}

/// Keep a pending replay query and its deadline alive while servicing inbound
/// frames and validity ticks. Neither traffic nor ticks may restart the read.
#[allow(clippy::too_many_arguments)] // one pump, one owner of each stream
async fn next_delivery(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    pool: &sqlx::PgPool,
    session: &auth::AuthSession,
    replay: &mut Replay,
    hub: &mut broadcast::Receiver<OutboxEntry>,
    typing: &mut Option<broadcast::Receiver<TypingSignal>>,
    validity_tick: &mut tokio::time::Interval,
) -> Option<Delivery> {
    let pending = async {
        if replay.done {
            match hub.recv().await {
                Ok(entry) => Some(Delivery::Event(entry)),
                Err(broadcast::error::RecvError::Lagged(_)) => Some(Delivery::Lagged),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        } else {
            match tokio::time::timeout(IO_TIMEOUT, replay.next(pool, session.user_id)).await {
                Ok(Ok(Some(entry))) => Some(Delivery::Event(entry)),
                Ok(Ok(None)) => Some(Delivery::ReplayEnd),
                _ => None,
            }
        }
    };
    tokio::pin!(pending);
    let mut completed = None;
    loop {
        if let Some(delivery) = completed.take() {
            return Some(delivery);
        }
        let mut typing_closed = false;
        tokio::select! {
            delivery = &mut pending => return delivery,
            // Best effort, like the replay-preserving control frames below:
            // relaying a signal never restarts a pending replay read.
            signal = async {
                match typing.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match signal {
                    Ok(signal) if signal.user_id != session.user_id
                        && signal.recipients.contains(&session.user_id) => {
                        let operation = async {
                            if !session_live(pool, session).await { return false; }
                            // Recipients were captured at publish; access is
                            // rechecked now, like events. Unknown/slow: drop it.
                            let visible = tokio::time::timeout(
                                IO_TIMEOUT,
                                conversation_visible(pool, session.user_id, signal.conversation_id),
                            ).await;
                            if !matches!(visible, Ok(true)) { return true; }
                            send_frame(sink, &ServerFrame::Typing {
                                conversation_id: signal.conversation_id,
                                user_id: signal.user_id,
                                last_seq: signal.last_seq,
                            }).await
                        };
                        if !with_delivery_progress(&mut pending, &mut completed, operation).await {
                            return None;
                        }
                    }
                    // Not for us, or skipped while lagging: typing is ephemeral.
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => typing_closed = true,
                }
            }
            _ = validity_tick.tick() => {
                if !with_delivery_progress(&mut pending, &mut completed, session_live(pool, session)).await {
                    return None;
                }
            }
            incoming = stream.next() => {
                let operation = async {
                if !session_live(pool, session).await { return false; }
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let frame = match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(ClientFrame::Heartbeat { seq }) => ServerFrame::HeartbeatAck { seq },
                            Ok(ClientFrame::Identify { .. }) => ServerFrame::Error { code: "already_identified" },
                            Err(_) => ServerFrame::Error { code: "bad_frame" },
                        };
                        if !send_frame(sink, &frame).await { return false; }
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return false,
                    _ => {}
                }
                true
                };
                if !with_delivery_progress(&mut pending, &mut completed, operation).await {
                    return None;
                }
            }
        }
        if typing_closed {
            *typing = None;
        }
    }
}

/// Keep releasing replay resources while a control operation waits for the
/// same pool. Buffer at most one delivery; validation must finish before it can
/// leave the pump. Read failure/deadline cancels the operation, failing closed.
async fn with_delivery_progress(
    pending: &mut (impl std::future::Future<Output = Option<Delivery>> + Unpin),
    completed: &mut Option<Delivery>,
    operation: impl std::future::Future<Output = bool>,
) -> bool {
    tokio::pin!(operation);
    loop {
        tokio::select! {
            valid = &mut operation => return valid,
            delivery = &mut *pending, if completed.is_none() => {
                let Some(delivery) = delivery else { return false; };
                *completed = Some(delivery);
            }
        }
    }
}

/// Recheck account session validity for each application frame and at bounded
/// idle intervals. Fail closed on timeout/database errors. Frames already in
/// flight at revocation are not retractable; this does not revoke MLS keys.
async fn session_live(pool: &sqlx::PgPool, session: &auth::AuthSession) -> bool {
    matches!(tokio::time::timeout(IO_TIMEOUT,
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL AND expires_at > statement_timestamp())"
        ).bind(session.session_id).bind(session.user_id).fetch_one(pool)
    ).await, Ok(Ok(true)))
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
    conversation_visible(pool, user_id, conversation).await
}

/// The live read gate behind [`event_visible`], shared with typing signals.
async fn conversation_visible(pool: &sqlx::PgPool, user_id: Uuid, conversation: Uuid) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_management::tests::{fixture, seed_user, session};
    use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn saturated_read(
        pool: sqlx::PgPool,
        session: auth::AuthSession,
        mut replay: Replay,
        barrier: std::sync::Arc<tokio::sync::Barrier>,
        started: tokio::sync::mpsc::Sender<()>,
    ) -> (bool, bool) {
        let pending = async {
            match tokio::time::timeout(IO_TIMEOUT, replay.next(&pool, session.user_id)).await {
                Ok(Ok(Some(entry))) => Some(Delivery::Event(entry)),
                _ => None,
            }
        };
        tokio::pin!(pending);
        tokio::select! {
            _ = &mut pending => panic!("locked replay cannot complete before validation"),
            _ = barrier.wait() => {}
        }
        let mut completed = None;
        let valid = with_delivery_progress(&mut pending, &mut completed, async {
            started.send(()).await.unwrap();
            session_live(&pool, &session).await
        })
        .await;
        let delivery = if valid {
            match completed {
                Some(delivery) => Some(delivery),
                None => pending.await,
            }
        } else {
            None
        };
        (valid, matches!(delivery, Some(Delivery::Event(_))))
    }
    async fn production_sized_pool(observer: &sqlx::PgPool) -> sqlx::PgPool {
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(observer)
            .await
            .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(IO_TIMEOUT)
            .after_connect(move |connection, _| {
                let schema = schema.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('search_path', $1, false), set_config('application_name', $1, false)")
                        .bind(schema).execute(connection).await?;
                    Ok(())
                })
            })
            .connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
        pool
    }

    // Exercise the shared control-operation boundary with the production pool
    // size. The observer never borrows application capacity, and barriers prove
    // all five reads hold connections before validation starts.
    async fn saturated_replay(mode: &str) {
        let (observer, _, user) = fixture()
            .await
            .expect("saturation regression requires DATABASE_URL");
        let pool = production_sized_pool(&observer).await;
        let peer = seed_user(&observer).await;
        let dm = messaging::find_or_create_dm(&observer, user, peer)
            .await
            .unwrap();
        messaging::send_message(
            &observer,
            peer,
            dm.id,
            Uuid::now_v7(),
            b"synthetic-saturated-replay",
            None,
        )
        .await
        .unwrap();
        let mut reads = Vec::new();
        let mut ids = Vec::new();
        for _ in 0..5 {
            let (id, token) = session(&observer, user).await;
            ids.push(id);
            reads.push((
                auth::authenticate(&observer, &token).await.unwrap(),
                Replay::start(&observer, user, None).await.unwrap(),
            ));
        }
        let mut blocker = observer.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("LOCK TABLE conversation_participants IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await
            .unwrap();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(6));
        let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(5);
        let mut tasks = Vec::new();
        for (session, replay) in reads {
            tasks.push(tokio::spawn(saturated_read(
                pool.clone(),
                session,
                replay,
                barrier.clone(),
                started_tx.clone(),
            )));
        }
        tokio::time::timeout(IO_TIMEOUT, async {
            loop {
                let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND datname = current_database() AND application_name = current_setting('application_name') AND wait_event_type = 'Lock' AND query LIKE 'SELECT o.id, o.topic, o.payload FROM outbox o%'")
                    .bind(pid).fetch_one(&observer).await.unwrap();
                if count == 5 { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("all production-sized pool connections held by replay");
        assert_eq!(pool.size(), 5);
        assert_eq!(pool.num_idle(), 0);
        match mode {
            "revoked" => {
                sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = ANY($1)")
                    .bind(&ids)
                    .execute(&observer)
                    .await
                    .unwrap();
            }
            "expired" => {
                sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second' WHERE id = ANY($1)")
                    .bind(&ids).execute(&observer).await.unwrap();
            }
            "valid" => {}
            _ => panic!("unknown synthetic mode"),
        }
        barrier.wait().await;
        for _ in 0..5 {
            tokio::time::timeout(IO_TIMEOUT, started_rx.recv())
                .await
                .unwrap()
                .unwrap();
        }
        blocker.commit().await.unwrap();
        for task in tasks {
            let result = tokio::time::timeout(IO_TIMEOUT, task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                result,
                (mode == "valid", mode == "valid"),
                "released replay must progress only for valid sessions"
            );
        }
        pool.close().await;
        observer.close().await;
    }

    #[tokio::test]
    async fn gateway_production_pool_saturation_preserves_valid_replay() {
        saturated_replay("valid").await;
    }

    #[tokio::test]
    async fn gateway_production_pool_saturation_denies_revoked_replay() {
        saturated_replay("revoked").await;
    }

    #[tokio::test]
    async fn gateway_production_pool_saturation_denies_expired_replay() {
        saturated_replay("expired").await;
    }

    #[tokio::test]
    async fn gateway_completed_replay_waits_for_control_validation() {
        for valid in [false, true] {
            let (released, release) = tokio::sync::oneshot::channel();
            let pending = async {
                released.send(()).unwrap();
                Some(Delivery::ReplayEnd)
            };
            tokio::pin!(pending);
            let mut completed = None;
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                with_delivery_progress(&mut pending, &mut completed, async {
                    release.await.unwrap();
                    tokio::task::yield_now().await;
                    valid
                }),
            )
            .await
            .expect("completed replay releases the waiting control operation");
            assert_eq!(result, valid, "buffering cannot override denial");
            assert!(matches!(completed, Some(Delivery::ReplayEnd)));
        }
    }

    #[tokio::test]
    async fn gateway_replay_deadline_interrupts_control_operation() {
        let pending = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            None
        };
        tokio::pin!(pending);
        let mut completed = None;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            with_delivery_progress(&mut pending, &mut completed, std::future::pending()),
        )
        .await;
        assert!(
            matches!(result, Ok(false)),
            "read deadline must remain polled"
        );
        assert!(completed.is_none());
    }

    async fn next(
        socket: &mut Socket,
    ) -> Option<Result<ClientMessage, tokio_tungstenite::tungstenite::Error>> {
        tokio::time::timeout(std::time::Duration::from_secs(7), socket.next())
            .await
            .expect("bounded gateway response")
    }

    async fn closed(socket: &mut Socket) {
        assert!(
            matches!(
                next(socket).await,
                None | Some(Err(_) | Ok(ClientMessage::Close(_)))
            ),
            "invalid session must receive no application frame"
        );
    }

    async fn identify(socket: &mut Socket) {
        socket
            .send(ClientMessage::Text(r#"{"op":"identify"}"#.into()))
            .await
            .unwrap();
        let Some(Ok(ClientMessage::Text(text))) = next(socket).await else {
            panic!("expected ready");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["op"],
            "ready"
        );
    }

    #[tokio::test]
    async fn gateway_revoked_or_expired_sessions_close_real_sockets() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::build_router(state))
                .await
                .unwrap();
        });
        // Upgrade is insufficient authorization: revocation before identify
        // suppresses Ready and all retained history.
        let (id, token) = session(&pool, user).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .unwrap();
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        socket
            .send(ClientMessage::Text(r#"{"op":"identify"}"#.into()))
            .await
            .unwrap();
        closed(&mut socket).await;
        assert!(
            connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
                .await
                .is_err()
        );

        // Quiet sockets close on periodic validation without client traffic.
        let (id, token) = session(&pool, user).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .unwrap();
        identify(&mut socket).await;
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        closed(&mut socket).await;

        // Expired sessions cannot elicit heartbeat ACKs after identification.
        let (id, token) = session(&pool, user).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .unwrap();
        identify(&mut socket).await;
        socket
            .send(ClientMessage::Text(r#"{"op":"heartbeat","seq":12}"#.into()))
            .await
            .unwrap();
        let Some(Ok(ClientMessage::Text(text))) = next(&mut socket).await else {
            panic!("heartbeat response");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["op"],
            "heartbeat_ack"
        );
        sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        socket
            .send(ClientMessage::Text(r#"{"op":"heartbeat","seq":13}"#.into()))
            .await
            .unwrap();
        closed(&mut socket).await;
        server.abort();
        pool.close().await;
    }

    #[tokio::test]
    async fn gateway_revocation_suppresses_live_and_replay_delivery() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let peer = seed_user(&pool).await;
        let dm = messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        let hub = state.hub.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::build_router(state))
                .await
                .unwrap();
        });
        let (id, token) = session(&pool, user).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .unwrap();
        identify(&mut socket).await;
        // Publish directly to this test's hub: no competing global outbox worker.
        messaging::send_message(
            &pool,
            peer,
            dm.id,
            Uuid::now_v7(),
            b"synthetic-envelope",
            None,
        )
        .await
        .unwrap();
        let (id_event, topic, payload): (Uuid, String, serde_json::Value) = sqlx::query_as("SELECT id, topic, payload FROM outbox WHERE payload->>'conversation_id' = $1 ORDER BY id DESC LIMIT 1")
            .bind(dm.id.to_string()).fetch_one(&pool).await.unwrap();
        let entry = OutboxEntry {
            id: id_event,
            topic,
            payload,
        };
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        let _ = hub.send(entry);
        closed(&mut socket).await;

        // Hold replay's initial outbox read, revoke while it is waiting, then
        // release it. The post-read check must suppress Ready and the event.
        let (id, token) = session(&pool, user).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .unwrap();
        let mut blocker = pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("LOCK TABLE outbox IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await
            .unwrap();
        socket
            .send(ClientMessage::Text(r#"{"op":"identify"}"#.into()))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND datname = current_database() AND application_name = current_setting('application_name') AND wait_event_type = 'Lock' AND query = 'SELECT id FROM outbox ORDER BY id DESC LIMIT 1')").bind(blocker_pid).fetch_one(&pool).await.unwrap();
                if waiting { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("gateway waiting for replay query");
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        blocker.commit().await.unwrap();
        closed(&mut socket).await;
        server.abort();

        pool.close().await;
    }

    #[tokio::test]
    async fn gateway_pending_replay_survives_ticks_and_heartbeats() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let peer = seed_user(&pool).await;
        let dm = messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        messaging::send_message(
            &pool,
            peer,
            dm.id,
            Uuid::now_v7(),
            b"synthetic-delayed-envelope",
            None,
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::build_router(state))
                .await
                .unwrap();
        });
        for revoke in [false, true] {
            let (id, token) = session(&pool, user).await;
            let mut blocker = pool.begin().await.unwrap();
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
            sqlx::query("LOCK TABLE conversation_participants IN ACCESS EXCLUSIVE MODE")
                .execute(&mut *blocker)
                .await
                .unwrap();
            let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
                .await
                .unwrap();
            identify(&mut socket).await;
            let wait_query = "SELECT pid, query_start FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND datname = current_database() AND application_name = current_setting('application_name') AND wait_event_type = 'Lock' AND query LIKE 'SELECT o.id, o.topic, o.payload FROM outbox o%'";
            let original: (i32, chrono::DateTime<chrono::Utc>) =
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        if let Some(row) = sqlx::query_as(wait_query)
                            .bind(blocker_pid)
                            .fetch_optional(&pool)
                            .await
                            .unwrap()
                        {
                            break row;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("replay waiting after Ready");
            socket
                .send(ClientMessage::Text(r#"{"op":"heartbeat","seq":7}"#.into()))
                .await
                .unwrap();
            let Some(Ok(ClientMessage::Text(text))) = next(&mut socket).await else {
                panic!("heartbeat during replay");
            };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&text).unwrap()["op"],
                "heartbeat_ack"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let current: (i32, chrono::DateTime<chrono::Utc>) = sqlx::query_as(wait_query)
                .bind(blocker_pid)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                original, current,
                "ticks and heartbeat must preserve the original pending query"
            );
            if revoke {
                sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
                    .bind(id)
                    .execute(&pool)
                    .await
                    .unwrap();
                // Periodic validation closes even while replay is still blocked.
                closed(&mut socket).await;
                blocker.commit().await.unwrap();
            } else {
                blocker.commit().await.unwrap();
                let Some(Ok(ClientMessage::Text(text))) = next(&mut socket).await else {
                    panic!("delayed valid replay must progress");
                };
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&text).unwrap()["op"],
                    "event"
                );
                socket.close(None).await.unwrap();
            }
        }
        server.abort();

        pool.close().await;
    }
    async fn typing_post(
        app: &axum::Router,
        token: &str,
        conversation: Uuid,
    ) -> axum::http::StatusCode {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri(format!("/v1/conversations/{conversation}/typing"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        tower::ServiceExt::oneshot(app.clone(), request)
            .await
            .unwrap()
            .status()
    }

    /// Give any queued frame time to arrive first, then require the heartbeat
    /// echo to be the very next frame: nothing else was sent to this socket.
    async fn nothing_before_heartbeat(socket: &mut Socket, seq: i64) {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        socket
            .send(ClientMessage::Text(
                format!(r#"{{"op":"heartbeat","seq":{seq}}}"#).into(),
            ))
            .await
            .unwrap();
        let Some(Ok(ClientMessage::Text(text))) = next(socket).await else {
            panic!("heartbeat response");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["op"],
            "heartbeat_ack",
            "no other frame may precede the echo"
        );
    }

    #[tokio::test]
    async fn gateway_relays_typing_only_to_other_live_members() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let peer = seed_user(&pool).await;
        let stranger = seed_user(&pool).await;
        let dm = messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        let app = crate::build_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut sockets = Vec::new();
        let mut tokens = Vec::new();
        for who in [peer, user, stranger] {
            let (_, token) = session(&pool, who).await;
            let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
                .await
                .unwrap();
            identify(&mut socket).await;
            sockets.push(socket);
            tokens.push(token);
        }

        assert_eq!(
            typing_post(&app, &tokens[1], dm.id).await,
            axum::http::StatusCode::NO_CONTENT
        );
        let Some(Ok(ClientMessage::Text(text))) = next(&mut sockets[0]).await else {
            panic!("typing frame for the other member");
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::json!({
                "op": "typing", "conversation_id": dm.id, "user_id": user, "last_seq": 0
            })
        );

        // A repeat inside the throttle interval relays nothing; the typist's
        // own socket and a stranger never see the signal at all.
        assert_eq!(
            typing_post(&app, &tokens[1], dm.id).await,
            axum::http::StatusCode::NO_CONTENT
        );
        for (seq, socket) in sockets.iter_mut().enumerate() {
            nothing_before_heartbeat(socket, i64::try_from(seq).unwrap()).await;
        }
        assert_eq!(
            typing_post(&app, &tokens[2], dm.id).await,
            axum::http::StatusCode::FORBIDDEN
        );
        server.abort();
        pool.close().await;
    }

    #[tokio::test]
    async fn channel_typing_uses_the_send_gate() {
        let Some((pool, state, owner)) = fixture().await else {
            return;
        };
        let guest = seed_user(&pool).await;
        let workspace = workspaces::create_workspace(&pool, owner, "Typing gate")
            .await
            .unwrap();
        let channel = workspaces::create_channel(&pool, owner, workspace.id, "typing")
            .await
            .unwrap();
        workspaces::add_member(&pool, owner, workspace.id, guest, workspaces::Role::Guest)
            .await
            .unwrap();
        let app = crate::build_router(state);
        let (_, owner_token) = session(&pool, owner).await;
        let (_, guest_token) = session(&pool, guest).await;
        // Guests read but cannot send, so they cannot announce typing either.
        assert_eq!(
            typing_post(&app, &guest_token, channel.conversation_id).await,
            axum::http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            typing_post(&app, &owner_token, channel.conversation_id).await,
            axum::http::StatusCode::NO_CONTENT
        );
        pool.close().await;
    }

    /// Serve `app` on a loopback port; returns the address and server task.
    async fn serve(app: axum::Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, server)
    }

    async fn connected(pool: &sqlx::PgPool, addr: std::net::SocketAddr, who: Uuid) -> Socket {
        let (_, token) = session(pool, who).await;
        let (mut socket, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .unwrap();
        identify(&mut socket).await;
        socket
    }

    /// The next typing frame, skipping any message events around it.
    async fn next_typing(socket: &mut Socket) -> serde_json::Value {
        loop {
            let Some(Ok(ClientMessage::Text(text))) = next(socket).await else {
                panic!("expected a typing frame");
            };
            let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
            if frame["op"] == "typing" {
                return frame;
            }
        }
    }

    #[tokio::test]
    async fn typing_frames_carry_the_latest_message_seq() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let peer = seed_user(&pool).await;
        let dm = messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        let app = crate::build_router(state);
        let (addr, server) = serve(app.clone()).await;
        let mut watcher = connected(&pool, addr, user).await;
        let (_, peer_token) = session(&pool, peer).await;
        messaging::send_message(&pool, peer, dm.id, Uuid::now_v7(), b"sealed", None)
            .await
            .unwrap();
        assert_eq!(
            typing_post(&app, &peer_token, dm.id).await,
            axum::http::StatusCode::NO_CONTENT
        );
        // A client that already loaded seq 1 from this typist keeps the
        // signal; one sent before that message would carry 0 and be dropped.
        assert_eq!(next_typing(&mut watcher).await["last_seq"], 1);
        server.abort();
        pool.close().await;
    }

    #[tokio::test]
    async fn typing_authorization_waits_for_a_concurrent_removal() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let peer = seed_user(&pool).await;
        let dm = messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        let app = crate::build_router(state);
        let (addr, server) = serve(app.clone()).await;
        let mut watcher = connected(&pool, addr, peer).await;
        let (_, token) = session(&pool, user).await;
        // A removal in flight: the typist's seat is deleted but not committed.
        let mut removal = pool.begin().await.unwrap();
        sqlx::query(
            "DELETE FROM conversation_participants WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(dm.id)
        .bind(user)
        .execute(&mut *removal)
        .await
        .unwrap();
        let request = tokio::spawn({
            let app = app.clone();
            async move { typing_post(&app, &token, dm.id).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !request.is_finished(),
            "authorization must wait for the removal instead of reading around it"
        );
        removal.commit().await.unwrap();
        assert_eq!(
            request.await.unwrap(),
            axum::http::StatusCode::FORBIDDEN,
            "the committed removal wins"
        );
        nothing_before_heartbeat(&mut watcher, 1).await;
        server.abort();
        pool.close().await;
    }

    #[tokio::test]
    async fn throttled_typing_repeats_skip_every_lookup() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let peer = seed_user(&pool).await;
        let dm = messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        let app = crate::build_router(state);
        let (addr, server) = serve(app.clone()).await;
        let mut watcher = connected(&pool, addr, peer).await;
        let (_, token) = session(&pool, user).await;
        assert_eq!(
            typing_post(&app, &token, dm.id).await,
            axum::http::StatusCode::NO_CONTENT
        );
        assert_eq!(next_typing(&mut watcher).await["user_id"], user.to_string());
        // Observable proof that a repeat inside the interval runs no query:
        // with the seat gone, any lookup would refuse it (403).
        sqlx::query(
            "DELETE FROM conversation_participants WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(dm.id)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            typing_post(&app, &token, dm.id).await,
            axum::http::StatusCode::NO_CONTENT,
            "admitted from memory, before any lookup"
        );
        nothing_before_heartbeat(&mut watcher, 1).await;
        server.abort();
        pool.close().await;
    }

    #[tokio::test]
    async fn typing_recipients_are_rechecked_at_delivery() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let peer = seed_user(&pool).await;
        let former = seed_user(&pool).await;
        let dm = messaging::find_or_create_dm(&pool, user, peer)
            .await
            .unwrap();
        let still = messaging::find_or_create_dm(&pool, user, former)
            .await
            .unwrap();
        let bus = TypingBus::new();
        let app = crate::routes::build_router_with_typing(state, bus.clone());
        let (addr, server) = serve(app).await;
        let mut removed = connected(&pool, addr, former).await;
        // Captured while `former` was still addressed (say, just before a
        // kick) and delivered after: the captured set alone is not trusted.
        bus.admit(user, dm.id)
            .expect("fresh slot")
            .publish(std::sync::Arc::from(vec![user, peer, former]), 0);
        bus.admit(user, still.id)
            .expect("fresh slot")
            .publish(std::sync::Arc::from(vec![user, former]), 0);
        assert_eq!(
            next_typing(&mut removed).await["conversation_id"],
            still.id.to_string(),
            "only the conversation it can still read arrives"
        );
        server.abort();
        pool.close().await;
    }
}
