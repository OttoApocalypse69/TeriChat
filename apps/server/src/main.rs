//! `TeriChat` modular-monolith API server (Alpha 0).
//!
//! Wiring only: [`main`] builds the [`config::Config`], connects the pool via
//! [`bootstrap`], shares [`state::AppState`], mounts [`routes::build_router`],
//! spawns the background workers, and serves with graceful shutdown. Probes
//! live in [`health`], shared errors in [`errors`].
//!
//! Serves [`health`](health::health) liveness, [`ready`](health::ready)
//! readiness, and the Milestone B account API under `/v1/auth/*` (register,
//! login, logout, device registration). Configuration comes from
//! [`config::Config`]; without `DATABASE_URL` the server boots bare with only
//! the probes live.

#![forbid(unsafe_code)]

mod auth;
mod bootstrap;
mod config;
mod errors;
mod gateway;
mod health;
mod messaging;
mod password;
mod routes;
mod state;
mod stats;
mod workspaces;

pub use bootstrap::{db_pool, MIGRATOR};
pub use routes::build_router;
pub use state::{AppState, HUB_CAPACITY};

use std::net::SocketAddr;

use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    let config = config::Config::from_env().unwrap_or_else(|err| {
        eprintln!("configuration error: {err}");
        std::process::exit(1);
    });

    bootstrap::init_tracing(&config);

    let pool = bootstrap::connect_pool(&config).await;

    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    bootstrap::spawn_workers(pool.as_ref(), &hub);

    let addr = SocketAddr::from((config.bind_addr, config.port));
    let listener = bootstrap::bind_listener(addr).await;
    tracing::info!(%addr, "terichat-server listening");
    axum::serve(listener, build_router(AppState { pool, hub }))
        .with_graceful_shutdown(bootstrap::shutdown_signal())
        .await
        .expect("serve axum router");
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use axum::{http::StatusCode, Router};
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn bare_state() -> AppState {
        let (hub, _) = broadcast::channel(HUB_CAPACITY);
        AppState { pool: None, hub }
    }

    async fn body_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

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

    async fn get_json(
        app: Router,
        uri: &str,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .oneshot(builder.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let (status, json) = body_json(build_router(bare_state()), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "terichat-server");
    }

    #[tokio::test]
    async fn ready_without_database_reports_not_configured() {
        let (status, json) = body_json(build_router(bare_state()), "/ready").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["database"], "not_configured");
    }

    #[tokio::test]
    async fn ready_with_unreachable_database_is_down() {
        let pool = db_pool("postgres://127.0.0.1:1/terichat_test").await;
        assert!(pool.is_err(), "expected connect failure, got {pool:?}");
    }

    #[tokio::test]
    async fn auth_routes_need_a_database() {
        // Probes-only boot: every account route reports 503, never 500.
        let app = build_router(bare_state());
        let (status, json) = post_json(
            app,
            "/v1/auth/register",
            serde_json::json!({"handle":"x","email":"x@y.z","display_name":"X","password":"pw"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"]["code"], "no_database");
    }

    #[tokio::test]
    async fn bearer_rejection_shapes() {
        // No database and no token alike: 401/503 JSON, never a bare status.
        let (status, json) = post_json(
            build_router(bare_state()),
            "/v1/auth/logout",
            serde_json::json!({}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"]["code"], "no_database");
    }

    #[test]
    fn migrator_embeds_identity_migration() {
        // Offline: the embedded migrator must resolve the known migrations.
        // The `migrate!` macro already fails the build when the directory is
        // missing or unparsable; this pins the expected content.
        assert!(
            !MIGRATOR.migrations.is_empty(),
            "expected at least the identity migration"
        );
        // Note: sqlx renders filename underscores as spaces in descriptions.
        for expected in ["identity", "messaging", "device agreement keys"] {
            assert!(
                MIGRATOR
                    .migrations
                    .iter()
                    .any(|migration| migration.description.contains(expected)),
                "expected a {expected} migration"
            );
        }
    }

    #[tokio::test]
    async fn migrations_apply_and_create_identity_tables() {
        // Requires a live database: `DATABASE_URL=... cargo test`. Prints a
        // skip (never a fake pass) when no database is configured, so plain
        // `cargo test` stays offline-clean for CI without services.
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: migrations_apply_and_create_identity_tables (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");

        for table in ["users", "devices", "sessions"] {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("query information_schema");
            assert!(exists, "expected table `{table}` after migrations");
        }
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Full HTTP
    /// account lifecycle through the real routes.
    #[tokio::test]
    async fn http_account_lifecycle() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: http_account_lifecycle (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = AppState {
            pool: Some(pool),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let handle = format!("httpbro{stamp}");
        let email = format!("httpbro{stamp}@example.com");

        // Register → 201 with the public user view (no secrets).
        let (status, user) = post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": handle, "email": email, "display_name": "Http Bro", "password": "s3cret-pw"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(user["handle"], handle.to_lowercase());
        assert!(user.get("password_hash").is_none());
        assert!(user.get("password").is_none());

        // Duplicate handle → 400, duplicate email → 400.
        let (status, body) = post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": handle, "email": "other@example.com", "display_name": "X", "password": "pw12"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "bad_request");
        let (status, _) = post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": "someother", "email": email, "display_name": "X", "password": "pw12"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Wrong password → 401 with the unauthorized envelope.
        let (status, body) = post_json(
            build_router(state.clone()),
            "/v1/auth/login",
            serde_json::json!({"handle": handle, "password": "wrong"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");

        // Login → 200 with a single-use token.
        let (status, login) = post_json(
            build_router(state.clone()),
            "/v1/auth/login",
            serde_json::json!({"handle": handle, "password": "s3cret-pw"}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let token = login["token"].as_str().expect("token issued").to_owned();

        // Logout → 200; the token is dead afterwards; garbage never works.
        let (status, _) = post_json(
            build_router(state.clone()),
            "/v1/auth/logout",
            serde_json::json!({}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = post_json(
            build_router(state.clone()),
            "/v1/auth/logout",
            serde_json::json!({}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");

        state.pool.as_ref().unwrap().close().await;
    }

    /// Requires a live database; skips honestly without one. Device keys over
    /// HTTP: real keys register (201), non-hex is rejected (400), and
    /// well-formed-but-degenerate keys are refused, never stored (400).
    #[tokio::test]
    async fn http_device_key_validation() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: http_device_key_validation (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let handle = format!("devkey{stamp}");
        post_json(
            build_router(state.clone()),
            "/v1/auth/register",
            serde_json::json!({"handle": handle, "email": format!("{handle}@example.com"), "display_name": "Keys", "password": "pw-keys-1"}),
            None,
        )
        .await;
        let (_, login) = post_json(
            build_router(state.clone()),
            "/v1/auth/login",
            serde_json::json!({"handle": handle, "password": "pw-keys-1"}),
            None,
        )
        .await;
        let token = login["token"].as_str().expect("token issued").to_owned();

        let device_keys = tericrypt::IdentityKeypair::generate().expect("device keys");
        let identity_hex = hex::encode(device_keys.identity_verify_key());
        let agree_hex = hex::encode(device_keys.agreement_pubkey());
        let (status, device) = post_json(
            build_router(state.clone()),
            "/v1/auth/devices",
            serde_json::json!({"label": "laptop", "identity_pubkey": identity_hex, "agreement_pubkey": agree_hex.clone()}),
            Some(&token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(device["label"], "laptop");
        assert_eq!(device["agreement_pubkey"], agree_hex);
        let zeros = "00".repeat(32);
        for (label, identity) in [("bad", "zz"), ("zero", zeros.as_str())] {
            let (status, _) = post_json(
                build_router(state.clone()),
                "/v1/auth/devices",
                serde_json::json!({"label": label, "identity_pubkey": identity, "agreement_pubkey": agree_hex}),
                Some(&token),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "label {label}");
        }
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The stats HTTP
    /// surface: `/v1/stats/me` serves only the caller's own rollup, the
    /// per-conversation endpoint is membership-checked, and missing
    /// credentials are rejected.
    #[tokio::test]
    async fn http_own_stats_is_scoped_to_caller() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: http_own_stats_is_scoped_to_caller (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let register = async |tag: &str| {
            auth::create_user(
                &pool,
                &format!("sthttp{tag}{stamp}"),
                &format!("sthttp{tag}{stamp}@example.com"),
                "Stats Http",
                "pw-stats-http",
            )
            .await
            .expect("register user")
        };
        let alice = register("a").await;
        let bob = register("b").await;
        let stranger = register("s").await;
        let token = async |handle: &str| {
            auth::login(&pool, handle, "pw-stats-http")
                .await
                .expect("login")
                .token
        };
        let (alice_token, bob_token, stranger_token) = (
            token(&alice.handle).await,
            token(&bob.handle).await,
            token(&stranger.handle).await,
        );
        let dm = messaging::find_or_create_dm(&pool, alice.id, bob.id)
            .await
            .expect("open dm");
        let (_, created) =
            messaging::send_message(&pool, alice.id, dm.id, Uuid::now_v7(), b"hi", None)
                .await
                .expect("send");
        assert!(created);
        // Fold the event the background worker would have folded.
        let event: (Uuid, String, serde_json::Value) = sqlx::query_as(
            "SELECT id, topic, payload FROM outbox
             WHERE (payload->>'conversation_id')::uuid = $1
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(dm.id)
        .fetch_one(&pool)
        .await
        .expect("outbox row");
        assert!(stats::process_event(&pool, event.0, &event.1, &event.2)
            .await
            .expect("fold"));

        // Alice reads her own rollup: one message.
        let (status, me) = get_json(
            build_router(state.clone()),
            "/v1/stats/me",
            Some(&alice_token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["message_count"], 1);
        assert_eq!(
            me["user_id"].as_str().expect("user uuid"),
            alice.id.to_string()
        );

        // Bob sees only his own (zero) rollup — no cross-user reads.
        let (status, bob_me) = get_json(
            build_router(state.clone()),
            "/v1/stats/me",
            Some(&bob_token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bob_me["message_count"], 0);

        // Conversation scope: a member reads their own count there.
        let uri = format!("/v1/stats/conversation?conversation_id={}", dm.id);
        let (status, scoped) =
            get_json(build_router(state.clone()), &uri, Some(&alice_token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(scoped["message_count"], 1);
        // An outsider gets 403, never a count.
        let (status, body) =
            get_json(build_router(state.clone()), &uri, Some(&stranger_token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["code"], "forbidden");

        // No credentials at all: 401.
        let (status, _) = get_json(build_router(state.clone()), "/v1/stats/me", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        pool.close().await;
    }

    /// Plain (non-TLS) client WebSocket used by the gateway tests.
    type WsStream = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

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

    async fn authed_json(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        token: &str,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let body = body.map_or_else(axum::body::Body::empty, |json| {
            axum::body::Body::from(json.to_string())
        });
        let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    /// Same as `authed_json` but for endpoints with empty bodies (`204 No
    /// Content`, or `201` without JSON): returns only the status.
    async fn authed_status(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        token: &str,
    ) -> StatusCode {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let body = body.map_or_else(axum::body::Body::empty, |json| {
            axum::body::Body::from(json.to_string())
        });
        app.oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
            .status()
    }

    /// Register three HTTP users; returns `(founder, member, stranger)` tokens.
    /// `prefix` keeps parallel tests on distinct handles.
    async fn http_users(
        pool: &sqlx::PgPool,
        stamp: &str,
        prefix: &str,
    ) -> (String, String, String) {
        for (tag, pw) in [
            ("phil", "pw-http-1"),
            ("bob", "pw-http-2"),
            ("mall", "pw-http-3"),
        ] {
            auth::create_user(
                pool,
                &format!("{prefix}{tag}{stamp}"),
                &format!("{prefix}{tag}{stamp}@example.com"),
                tag,
                pw,
            )
            .await
            .expect("register user");
        }
        let login = |handle: String, pw: &'static str| {
            let pool = pool.clone();
            async move { auth::login(&pool, &handle, pw).await.expect("login").token }
        };
        // Sequential awaits: no lifetime escapes the closure call.
        let phil = login(format!("{prefix}phil{stamp}"), "pw-http-1").await;
        let bob = login(format!("{prefix}bob{stamp}"), "pw-http-2").await;
        let mallory = login(format!("{prefix}mall{stamp}"), "pw-http-3").await;
        (phil, bob, mallory)
    }

    /// Create a workspace plus its `#general` channel through the routes.
    /// Returns `(workspace_id, conversation_id)`.
    async fn http_space(state: &AppState, token: &str) -> (String, String) {
        let (status, workspace) = post_json(
            build_router(state.clone()),
            "/v1/workspaces",
            serde_json::json!({ "name": "HTTP Space" }),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(workspace["my_role"], "owner");
        let workspace_id = workspace["id"].as_str().expect("workspace id").to_owned();
        let (status, channel) = post_json(
            build_router(state.clone()),
            &format!("/v1/workspaces/{workspace_id}/channels"),
            serde_json::json!({ "name": "general" }),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let conversation_id = channel["conversation_id"]
            .as_str()
            .expect("convo id")
            .to_owned();
        (workspace_id, conversation_id)
    }

    /// Requires a live database; skips honestly without one. Milestone E happy
    /// path over real routes: workspace → channel → invite → join → channel
    /// send → history.
    #[tokio::test]
    async fn workspace_http_channel_send() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_http_channel_send (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "ha").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        // Single-use invite admits Bob exactly once.
        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({ "max_uses": 1 }),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("invite code").to_owned();

        let (status, joined) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(joined["my_role"], "member");

        // Bob sends through the channel's backing conversation; both read it.
        let (status, sent) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"channel-bro"),
            }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(sent["seq"], 1);

        let (status, history) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(history.as_array().expect("history").len(), 1);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The hostile side
    /// over real routes: strangers hit the same wall everywhere, spent codes
    /// stay silent, and bans go dark immediately.
    #[tokio::test]
    async fn workspace_http_outsider_ban_audit() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_http_outsider_ban_audit (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, mallory) = http_users(&pool, &stamp, "hb").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        // Open invite admits Bob; Mallory stays a stranger.
        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("invite code").to_owned();
        let (status, _) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{workspace_id}"),
            None,
            &mallory,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"mallory-hi"),
            }),
            Some(&mallory),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Founder bans Bob: his history goes dark, his sends die.
        let bob_id = auth::user_id_by_handle(&pool, &format!("hbbob{stamp}"))
            .await
            .expect("bob id");
        let status = authed_status(
            build_router(state()),
            "POST",
            &format!("/v1/workspaces/{workspace_id}/bans"),
            Some(serde_json::json!({ "user_id": bob_id, "reason": "spam" })),
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Audit is founder-visible and carries no envelope bytes.
        let (status, audit) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{workspace_id}/audit?limit=50"),
            None,
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let entries = audit.as_array().expect("audit entries");
        assert!(entries.len() >= 5, "audit has {} entries", entries.len());
        let actions: Vec<&str> = entries
            .iter()
            .filter_map(|entry| entry["action"].as_str())
            .collect();
        assert!(actions.contains(&"member.banned"));
        assert!(actions.contains(&"invite.accepted"));
        for entry in entries {
            assert!(!entry.to_string().contains("ciphertext"));
        }
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. No oracles, no
    /// precedence leaks: strangers get the same workspace-shaped 403 for
    /// unknown handles, unknown workspaces, and undecodable sends.
    #[tokio::test]
    async fn workspace_member_oracle() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_member_oracle (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "hd").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        // Unknown handle as a stranger: 403, not the 400 members get.
        let (status, body) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/members"),
            serde_json::json!({"user_handle": "nobody-here", "role": "member"}),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["message"], "not a workspace member");

        // Unknown workspace id: identical wall, no existence signal.
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{}", Uuid::now_v7()),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Kicked ex-member knows the conversation id: garbage bytes still
        // meet the membership wall first (403), not a decode error (400).
        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("code").to_owned();
        let (status, _) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let bob_id = auth::user_id_by_handle(&pool, &format!("hdbob{stamp}"))
            .await
            .expect("bob id");
        let status = authed_status(
            build_router(state()),
            "POST",
            &format!("/v1/workspaces/{workspace_id}/bans"),
            Some(serde_json::json!({ "user_id": bob_id })),
            &phil,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": "!!!not-base64!!!",
            }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. Guests are
    /// read-only, not blind: history reads succeed, sends are refused, and a
    /// stranded seat heals on the next read instead of locking them out.
    #[tokio::test]
    async fn workspace_guest_reads_but_not_writes() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_guest_reads_but_not_writes (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "he").await;
        let (workspace_id, conversation_id) = http_space(&state(), &phil).await;

        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({ "initial_role": "guest" }),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("code").to_owned();
        let (status, joined) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(joined["my_role"], "guest");

        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"owner-says-hi"),
            }),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        // Guest reads fine…
        let (status, history) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(history.as_array().expect("history").len(), 1);
        // …but cannot write.
        let (status, _) = post_json(
            build_router(state()),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": conversation_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(b"guest-tries"),
            }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Strand the guest's seat; the next read heals and succeeds.
        let bob_id = auth::user_id_by_handle(&pool, &format!("hebob{stamp}"))
            .await
            .expect("bob id");
        sqlx::query(
            "DELETE FROM conversation_participants
             WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(conversation_id.parse::<Uuid>().expect("convo uuid"))
        .bind(bob_id)
        .execute(&pool)
        .await
        .expect("strand guest");
        let (status, history) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/messages?conversation_id={conversation_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(history.as_array().expect("history").len(), 1);
        pool.close().await;
    }

    /// Requires a live database; skips honestly without one. The leave path:
    /// a member exits voluntarily (204, then 403 everywhere); the last owner
    /// is stopped with a 400.
    #[tokio::test]
    async fn workspace_leave_route() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: workspace_leave_route (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        let state = || AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (phil, bob, _) = http_users(&pool, &stamp, "hf").await;
        let (workspace_id, _) = http_space(&state(), &phil).await;

        let (status, invite) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/invites"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let code = invite["code"].as_str().expect("code").to_owned();
        let (status, _) = post_json(
            build_router(state()),
            "/v1/workspaces/join",
            serde_json::json!({ "code": code }),
            Some(&bob),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let status = authed_status(
            build_router(state()),
            "POST",
            &format!("/v1/workspaces/{workspace_id}/leave"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = authed_json(
            build_router(state()),
            "GET",
            &format!("/v1/workspaces/{workspace_id}"),
            None,
            &bob,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = post_json(
            build_router(state()),
            &format!("/v1/workspaces/{workspace_id}/leave"),
            serde_json::json!({}),
            Some(&phil),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        pool.close().await;
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

    /// Requires a live database; skips honestly without one. The Milestone D
    /// proof: a sealed `TeriCrypt` envelope travels the real send/history
    /// path as opaque bytes and only the recipient device opens it. The
    /// server never sees `bro`.
    #[tokio::test]
    async fn sealed_dm_end_to_end() {
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: sealed_dm_end_to_end (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let (alice_token, _, bob_id, dm_id) = ws_fixture(&pool, &stamp).await;
        let alice_id = auth::user_id_by_handle(&pool, &format!("wsalice{stamp}"))
            .await
            .expect("alice id");

        // Device keypairs live client-side; only the public halves register.
        let alice_dev = tericrypt::IdentityKeypair::generate().expect("alice device keys");
        let bob_dev = tericrypt::IdentityKeypair::generate().expect("bob device keys");
        let mallory_dev = tericrypt::IdentityKeypair::generate().expect("mallory device keys");
        auth::register_device(
            &pool,
            alice_id,
            "alice-phone",
            alice_dev.identity_verify_key(),
            alice_dev.agreement_pubkey(),
        )
        .await
        .expect("register alice device");
        auth::register_device(
            &pool,
            bob_id,
            "bob-laptop",
            bob_dev.identity_verify_key(),
            bob_dev.agreement_pubkey(),
        )
        .await
        .expect("register bob device");

        // Alice seals `bro` to Bob's agreement key and sends the wire bytes.
        let envelope =
            tericrypt::seal(&alice_dev, &bob_dev.agreement_pubkey(), b"bro").expect("seal");
        let wire = envelope.to_bytes();
        assert!(wire.len() >= tericrypt::HEADER_LEN);
        let http_state = AppState {
            pool: Some(pool.clone()),
            hub: broadcast::channel(HUB_CAPACITY).0,
        };
        let (status, sent) = post_json(
            build_router(http_state),
            "/v1/messages",
            serde_json::json!({
                "conversation_id": dm_id,
                "client_msg_id": Uuid::now_v7(),
                "ciphertext_b64": STANDARD.encode(&wire),
            }),
            Some(&alice_token),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        // Bob reads history and opens the envelope; Mallory's device cannot.
        let history = messaging::message_history(&pool, bob_id, dm_id, 0, 50)
            .await
            .expect("bob history");
        assert_eq!(history.len(), 1);
        let received =
            tericrypt::SealedEnvelope::from_bytes(&history[0].ciphertext).expect("parse envelope");
        let plaintext = tericrypt::open(&bob_dev, &alice_dev.identity_verify_key(), &received)
            .expect("bob opens");
        assert_eq!(plaintext, b"bro");
        assert!(
            tericrypt::open(&mallory_dev, &alice_dev.identity_verify_key(), &received).is_err()
        );

        // The stored bytes are the sealed envelope, not plaintext.
        assert_eq!(history[0].ciphertext, wire);
        assert!(sent["seq"].as_i64().unwrap() >= 1);
        pool.close().await;
    }
}
