//! Realtime gateway integration over the real router and database.
//!
//! Moved verbatim from `src/main.rs` integration module.
#![forbid(unsafe_code)]
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use http_body_util::BodyExt;
use terichat_server::{
    auth, build_router, db_pool, messaging, workspaces, AppState, HUB_CAPACITY, MIGRATOR,
};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

async fn post_json(
    app: Router,
    uri: &str,
    body: serde_json::Value,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .oneshot(
            builder
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (status, json)
}

/// Plain (non-TLS) client WebSocket used by the gateway tests.
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Register + log in two users and open their DM. Returns
/// `(alice_token, bob_token, bob_id, dm_id)`.
async fn ws_fixture(pool: &sqlx::PgPool, tag: &str) -> (String, String, Uuid, Uuid) {
    let alice = auth::create_user(
        pool,
        &format!("wsalice{tag}"),
        &format!("wsalice{tag}@example.com"),
        "WsAlice",
        "pw-alice-ws",
    )
    .await
    .expect("register alice");
    let bob = auth::create_user(
        pool,
        &format!("wsbob{tag}"),
        &format!("wsbob{tag}@example.com"),
        "WsBob",
        "pw-bob-ws",
    )
    .await
    .expect("register bob");
    let alice_token = auth::login(pool, &alice.handle, "pw-alice-ws")
        .await
        .expect("login alice")
        .token;
    let bob_token = auth::login(pool, &bob.handle, "pw-bob-ws")
        .await
        .expect("login bob")
        .token;
    let dm = messaging::find_or_create_dm(pool, alice.id, bob.id)
        .await
        .expect("open dm");
    (alice_token, bob_token, bob.id, dm.id)
}

/// Send one message through the HTTP route (oneshot consumes routers,
/// so each call builds a fresh one from shared state).
async fn http_send(
    state: &AppState,
    token: &str,
    dm: Uuid,
    plaintext_b64: &str,
) -> (StatusCode, serde_json::Value) {
    post_json(
        build_router(state.clone()),
        "/v1/messages",
        serde_json::json!({
            "conversation_id": dm,
            "client_msg_id": Uuid::now_v7(),
            "ciphertext_b64": plaintext_b64,
        }),
        Some(token),
    )
    .await
}

use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// The outbox table is global to the test database and each realtime test
/// spawns a worker that claims rows for its own hub: running two such
/// tests concurrently lets the workers steal each other's live events.
/// This guard serializes them; replay-only tests are unaffected.
static WS_SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

/// Hold while a realtime test owns the shared outbox worker pattern.
async fn ws_guard() -> tokio::sync::MutexGuard<'static, ()> {
    WS_SERIAL
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

/// Send a JSON frame on a gateway test socket.
async fn ws_send(ws: &mut WsStream, value: serde_json::Value) {
    ws.send(WsMessage::Text(value.to_string().into()))
        .await
        .expect("send ws frame");
}

/// Read the next text frame as JSON (15 s budget).
async fn ws_next(ws: &mut WsStream) -> serde_json::Value {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(15), ws.next())
        .await
        .expect("ws frame in time")
        .expect("ws stream open");
    match frame.expect("ws message ok") {
        WsMessage::Text(text) => serde_json::from_str(&text).expect("json frame"),
        other => panic!("expected text frame, got {other:?}"),
    }
}

/// Identify and expect the next frame back.
async fn ws_identify(ws: &mut WsStream, resume_after: Option<Uuid>) -> serde_json::Value {
    ws_send(
        ws,
        serde_json::json!({"op": "identify", "resume_after": resume_after}),
    )
    .await;
    ws_next(ws).await
}

/// Connect, identify, and expect `ready`. Returns the live socket.
async fn connect_identified(
    addr: &std::net::SocketAddr,
    token: &str,
    resume_after: Option<Uuid>,
) -> WsStream {
    use tokio_tungstenite::connect_async;
    let (mut ws, _) = connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
        .await
        .expect("ws connect");
    assert_eq!(ws_identify(&mut ws, resume_after).await["op"], "ready");
    ws
}

/// Pull the event id out of an `event` frame.
fn event_id_of(frame: &serde_json::Value) -> Uuid {
    frame["event"]["event_id"]
        .as_str()
        .expect("event id")
        .parse()
        .expect("event uuid")
}

/// Read one frame and assert it is the expected conversation event.
/// Returns the event id for resume chaining.
async fn expect_event(ws: &mut WsStream, seq: i64) -> Uuid {
    let frame = ws_next(ws).await;
    assert_eq!(frame["op"], "event");
    assert_eq!(frame["event"]["payload"]["data"]["seq"], seq);
    event_id_of(&frame)
}

/// Register one gateway user and return `(token, id)`.
async fn gw_login(pool: &sqlx::PgPool, tag: &str, stamp: &str) -> (String, Uuid) {
    let user = auth::create_user(
        pool,
        &format!("{tag}{stamp}"),
        &format!("{tag}{stamp}@example.com"),
        tag,
        "pw-gw-1",
    )
    .await
    .expect("register user");
    let token = auth::login(pool, &format!("{tag}{stamp}"), "pw-gw-1")
        .await
        .expect("login")
        .token;
    (token, user.id)
}

/// Requires a live database; skips honestly without one. The Alpha 0
/// milestone: two clients exchange an encrypted DM over the realtime
/// gateway, reconnect, and resume without loss or duplicates. The first
/// encrypted message is `bro` — opaque to the server, decoded only by
/// this test.
#[tokio::test]
async fn two_clients_exchange_and_resume() {
    let _serial = ws_guard().await;
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: two_clients_exchange_and_resume (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let (alice_token, bob_token, bob_id, dm_id) = ws_fixture(&pool, &stamp).await;

    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    let state = AppState {
        pool: Some(pool.clone()),
        hub: hub.clone(),
    };
    // HTTP posts only need the pool; each oneshot call builds its own router.
    let http_state = AppState {
        pool: Some(pool.clone()),
        hub: broadcast::channel(HUB_CAPACITY).0,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("test addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state))
            .await
            .expect("serve test app");
    });
    let _worker = tokio::spawn(messaging::outbox_worker(pool.clone(), hub));

    let _alice_ws = connect_identified(&addr, &alice_token, None).await;
    let mut bob_ws = connect_identified(&addr, &bob_token, None).await;

    // Alice sends `bro` through the HTTP route; Bob gets the live event.
    let bro_b64 = STANDARD.encode(b"bro");
    let (status, sent) = http_send(&http_state, &alice_token, dm_id, &bro_b64).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(sent["deduped"], false);
    assert_eq!(sent["seq"], 1);

    let event1_id = expect_event(&mut bob_ws, 1).await;

    // Bob reads history: the opaque bytes decode to `bro` client-side.
    let history = messaging::message_history(&pool, bob_id, dm_id, 0, 50)
        .await
        .expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].ciphertext, b"bro");

    // Second message flows live too.
    let second_b64 = STANDARD.encode(b"second");
    let (status, _) = http_send(&http_state, &alice_token, dm_id, &second_b64).await;
    assert_eq!(status, StatusCode::CREATED);
    let event2_id = expect_event(&mut bob_ws, 2).await;
    assert_ne!(event1_id, event2_id);

    // Bob drops, Alice sends a third, Bob resumes after the second:
    // exactly one replayed event, no duplicates.
    bob_ws.close(None).await.expect("bob close");
    drop(bob_ws);
    let third_b64 = STANDARD.encode(b"third");
    let (status, _) = http_send(&http_state, &alice_token, dm_id, &third_b64).await;
    assert_eq!(status, StatusCode::CREATED);

    let mut bob_ws = connect_identified(&addr, &bob_token, Some(event2_id)).await;
    let _replayed_id = expect_event(&mut bob_ws, 3).await;
    let quiet = tokio::time::timeout(std::time::Duration::from_secs(2), bob_ws.next()).await;
    assert!(
        quiet.is_err(),
        "expected no duplicate delivery, got {quiet:?}"
    );

    server.abort();
    pool.close().await;
}

#[tokio::test]
async fn gateway_replay_drains_all_pages_and_quiet_conversation() {
    let url =
        std::env::var("DATABASE_URL").expect("service-enabled regression requires DATABASE_URL");
    let pool = db_pool(&url).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let tag = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap()
        .to_string();
    let (owner_token, token, user, dm) = ws_fixture(&pool, &tag).await;
    let quiet = messaging::create_conversation(&pool, user, "group", &[])
        .await
        .unwrap()
        .id;
    let owner = auth::authenticate(&pool, &owner_token)
        .await
        .unwrap()
        .user_id;
    let hidden_ws = workspaces::create_workspace(&pool, owner, "hidden")
        .await
        .unwrap();
    let hidden = workspaces::create_channel(&pool, owner, hidden_ws.id, "hidden")
        .await
        .unwrap();
    sqlx::query("INSERT INTO conversation_participants(conversation_id,user_id) VALUES ($1,$2)")
        .bind(hidden.conversation_id)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    for _ in 0..101 {
        sqlx::query("INSERT INTO outbox(id,topic,payload,published_at) VALUES ($1,'message.created',$2,now())")
            .bind(Uuid::now_v7()).bind(serde_json::json!({"conversation_id":hidden.conversation_id,"data":{"seq":-1}}))
            .execute(&pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO outbox(id,topic,payload,published_at) VALUES ($1,'message.created',$2,now())",
    )
    .bind(Uuid::now_v7())
    .bind(serde_json::json!({"conversation_id":"not-a-uuid"}))
    .execute(&pool)
    .await
    .unwrap();
    for seq in 1..=102 {
        let conversation = if seq == 102 { quiet } else { dm };
        sqlx::query("INSERT INTO outbox(id,topic,payload,published_at) VALUES ($1,'message.created',$2,now())")
            .bind(Uuid::now_v7())
            .bind(serde_json::json!({"conversation_id":conversation,"data":{"seq":seq}}))
            .execute(&pool)
            .await
            .unwrap();
    }
    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    let state = AppState {
        pool: Some(pool.clone()),
        hub,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    let mut ws = connect_identified(&addr, &token, None).await;
    for seq in 1..=102 {
        expect_event(&mut ws, seq).await;
    }
    ws_send(&mut ws, serde_json::json!({"op":"heartbeat","seq":77})).await;
    assert_eq!(ws_next(&mut ws).await["op"], "heartbeat_ack");
    ws.close(None).await.unwrap();
    server.abort();
    pool.close().await;
}

#[tokio::test]
async fn gateway_live_delivers_reversed_commits() {
    let url =
        std::env::var("DATABASE_URL").expect("service-enabled regression requires DATABASE_URL");
    let pool = db_pool(&url).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let tag = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap()
        .to_string();
    let (_, token, _, dm) = ws_fixture(&pool, &tag).await;
    let low = Uuid::now_v7();
    let high = Uuid::now_v7();
    assert!(low < high);
    let entry = |id, seq| messaging::OutboxEntry {
        id,
        topic: "message.created".into(),
        payload: serde_json::json!({"conversation_id":dm,"data":{"seq":seq}}),
    };
    let low_entry = entry(low, 1);
    let high_entry = entry(high, 2);
    let mut first = pool.begin().await.unwrap();
    let mut second = pool.begin().await.unwrap();
    for (tx, event) in [(&mut first, &low_entry), (&mut second, &high_entry)] {
        sqlx::query("INSERT INTO outbox(id,topic,payload,published_at) VALUES ($1,$2,$3,now())")
            .bind(event.id)
            .bind(&event.topic)
            .bind(&event.payload)
            .execute(&mut **tx)
            .await
            .unwrap();
    }
    second.commit().await.unwrap();
    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    let state = AppState {
        pool: Some(pool.clone()),
        hub: hub.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    let mut ws = connect_identified(&addr, &token, None).await;
    assert_eq!(expect_event(&mut ws, 2).await, high);
    first.commit().await.unwrap();
    hub.send(low_entry).unwrap();
    assert_eq!(expect_event(&mut ws, 1).await, low);
    hub.send(high_entry).unwrap();
    ws_send(&mut ws, serde_json::json!({"op":"heartbeat","seq":19})).await;
    assert_eq!(ws_next(&mut ws).await["op"], "heartbeat_ack");
    ws.close(None).await.unwrap();
    server.abort();
    pool.close().await;
}

#[tokio::test]
async fn gateway_lag_recovers_more_than_one_page() {
    let url =
        std::env::var("DATABASE_URL").expect("service-enabled regression requires DATABASE_URL");
    let pool = db_pool(&url).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let tag = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap()
        .to_string();
    let (_, token, user, dm) = ws_fixture(&pool, &tag).await;
    let quiet = messaging::create_conversation(&pool, user, "group", &[])
        .await
        .unwrap()
        .id;
    let (hub, _) = broadcast::channel(1);
    let state = AppState {
        pool: Some(pool.clone()),
        hub: hub.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    let mut ws = connect_identified(&addr, &token, None).await;
    for seq in 1..=102 {
        let conversation = if seq == 102 { quiet } else { dm };
        sqlx::query("INSERT INTO outbox(id,topic,payload,published_at) VALUES ($1,'message.created',$2,now())")
            .bind(Uuid::now_v7()).bind(serde_json::json!({"conversation_id":conversation,"data":{"seq":seq}}))
            .execute(&pool).await.unwrap();
    }
    // Current-thread runtime, no await between sends: capacity one must
    // report Lagged before either frame can be consumed by the server.
    for _ in 0..2 {
        hub.send(messaging::OutboxEntry {
            id: Uuid::now_v7(),
            topic: "irrelevant".into(),
            payload: serde_json::json!({}),
        })
        .unwrap();
    }
    for seq in 1..=102 {
        expect_event(&mut ws, seq).await;
    }
    ws.close(None).await.unwrap();
    server.abort();
    pool.close().await;
}

#[tokio::test]
async fn gateway_identify_has_a_deadline() {
    let url =
        std::env::var("DATABASE_URL").expect("service-enabled regression requires DATABASE_URL");
    let pool = db_pool(&url).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let tag = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap()
        .to_string();
    let (_, token, _, _) = ws_fixture(&pool, &tag).await;
    let state = AppState {
        pool: Some(pool.clone()),
        hub: broadcast::channel(1).0,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/v1/gateway?token={token}"))
            .await
            .unwrap();
    let closed = tokio::time::timeout(std::time::Duration::from_secs(7), ws.next()).await;
    assert!(
        closed.is_ok(),
        "unidentified connection must be closed within the deadline"
    );
    assert!(!matches!(closed.unwrap(), Some(Ok(WsMessage::Text(_)))));
    server.abort();
    pool.close().await;
}

/// Requires a live database; skips honestly without one. Gateway parity
/// with HTTP: a guest replays channel events it may read, and a banned
/// member's open connection goes quiet — kicks take effect live, without
/// a reconnect.
#[tokio::test]
async fn gateway_channel_guest_and_ban() {
    let _serial = ws_guard().await;
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: gateway_channel_guest_and_ban (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let (phil_token, phil_id) = gw_login(&pool, "gwphil", &stamp).await;
    let (guest_token, guest_id) = gw_login(&pool, "gwguest", &stamp).await;
    let (victim_token, victim_id) = gw_login(&pool, "gwvic", &stamp).await;

    let workspace = workspaces::create_workspace(&pool, phil_id, "GW Space")
        .await
        .expect("create workspace");
    workspaces::add_member(
        &pool,
        phil_id,
        workspace.id,
        guest_id,
        workspaces::Role::Guest,
    )
    .await
    .expect("add guest");
    workspaces::add_member(
        &pool,
        phil_id,
        workspace.id,
        victim_id,
        workspaces::Role::Member,
    )
    .await
    .expect("add victim");
    let channel = workspaces::create_channel(&pool, phil_id, workspace.id, "general")
        .await
        .expect("create channel");

    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    let state = AppState {
        pool: Some(pool.clone()),
        hub: hub.clone(),
    };
    let http_state = AppState {
        pool: Some(pool.clone()),
        hub: broadcast::channel(HUB_CAPACITY).0,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("test addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, build_router(state))
            .await
            .expect("serve test app");
    });
    let _worker = tokio::spawn(messaging::outbox_worker(pool.clone(), hub));

    http_send(
        &http_state,
        &phil_token,
        channel.conversation_id,
        &STANDARD.encode(b"one"),
    )
    .await;
    // Guest replays what it may read; victim replays as a member.
    workspaces::ensure_all_participation(&pool, guest_id)
        .await
        .expect("heal guest before replay");
    let mut guest_ws = connect_identified(&addr, &guest_token, None).await;
    assert_eq!(
        ws_next(&mut guest_ws).await["event"]["payload"]["data"]["seq"],
        1
    );
    let mut victim_ws = connect_identified(&addr, &victim_token, None).await;
    assert_eq!(
        ws_next(&mut victim_ws).await["event"]["payload"]["data"]["seq"],
        1
    );

    // Ban lands live: the victim's open connection goes quiet while the
    // guest's keeps streaming.
    workspaces::ban_member(&pool, phil_id, workspace.id, victim_id, "spam")
        .await
        .expect("ban victim");
    http_send(
        &http_state,
        &phil_token,
        channel.conversation_id,
        &STANDARD.encode(b"two"),
    )
    .await;
    expect_event(&mut guest_ws, 2).await;
    let quiet =
        tokio::time::timeout(std::time::Duration::from_secs(3), ws_next(&mut victim_ws)).await;
    assert!(quiet.is_err(), "banned connection must go quiet");

    server.abort();
    pool.close().await;
}
