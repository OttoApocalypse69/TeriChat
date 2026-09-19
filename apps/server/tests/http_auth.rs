//! HTTP account/device/stats lifecycle plus sealed-DM proof.
//!
//! Moved verbatim from `src/main.rs` integration module.
#![forbid(unsafe_code)]
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use http_body_util::BodyExt;
use terichat_server::{
    auth, build_router, db_pool, messaging, stats, AppState, HUB_CAPACITY, MIGRATOR,
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

async fn get_json(app: Router, uri: &str, token: Option<&str>) -> (StatusCode, serde_json::Value) {
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
    let state = AppState::new(
        Some(pool),
        broadcast::channel(HUB_CAPACITY).0,
        std::env::temp_dir().join("terichat-test-attachments"),
    );

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
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        std::env::temp_dir().join("terichat-test-attachments"),
    );

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
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        std::env::temp_dir().join("terichat-test-attachments"),
    );

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
    let (_, created) = messaging::send_message(&pool, alice.id, dm.id, Uuid::now_v7(), b"hi", None)
        .await
        .expect("send");
    assert!(created);
    // Fold the event the background worker would have folded.
    let event: (Uuid, String, serde_json::Value) = sqlx::query_as(
        "SELECT id, topic, payload FROM outbox
         WHERE payload->>'conversation_id' = CAST($1 AS UUID)::text
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
    let (status, scoped) = get_json(build_router(state.clone()), &uri, Some(&alice_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scoped["message_count"], 1);
    // An outsider gets 403, never a count.
    let (status, body) = get_json(build_router(state.clone()), &uri, Some(&stranger_token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "forbidden");

    // No credentials at all: 401.
    let (status, _) = get_json(build_router(state.clone()), "/v1/stats/me", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

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
    let envelope = tericrypt::seal(&alice_dev, &bob_dev.agreement_pubkey(), b"bro").expect("seal");
    let wire = envelope.to_bytes();
    assert!(wire.len() >= tericrypt::HEADER_LEN);
    let http_state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        std::env::temp_dir().join("terichat-test-attachments"),
    );
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
    let plaintext =
        tericrypt::open(&bob_dev, &alice_dev.identity_verify_key(), &received).expect("bob opens");
    assert_eq!(plaintext, b"bro");
    assert!(tericrypt::open(&mallory_dev, &alice_dev.identity_verify_key(), &received).is_err());

    // The stored bytes are the sealed envelope, not plaintext.
    assert_eq!(history[0].ciphertext, wire);
    assert!(sent["seq"].as_i64().unwrap() >= 1);
    pool.close().await;
}
