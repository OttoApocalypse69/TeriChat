//! Attachment HTTP regressions: membership-oracle, caps, byte round-trip.
//!
//! Requires a live database (`DATABASE_URL=... cargo test`); skips honestly
//! without one. Filesystem bytes land in a per-test temp dir.
#![forbid(unsafe_code)]
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use terichat_server::{auth, build_router, db_pool, messaging, AppState, HUB_CAPACITY, MIGRATOR};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

fn test_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "terichat-attach-test-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

async fn post_bytes(
    app: Router,
    uri: &str,
    token: &str,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> (StatusCode, Vec<u8>) {
    let uri = format!(
        "{uri}?filename={}&mime={}",
        urlencoding(filename),
        urlencoding(mime)
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/octet-stream")
                .body(axum::body::Body::from(bytes.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

async fn get_bytes(app: Router, uri: &str, token: &str) -> (StatusCode, Vec<u8>, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec(), content_type)
}

fn urlencoding(raw: &str) -> String {
    raw.replace(' ', "%20").replace('/', "%2F")
}

/// Alice uploads, Bob downloads byte-identical, stranger sees 404.
#[tokio::test]
async fn attachment_round_trip_and_no_oracle() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: attachment_round_trip_and_no_oracle (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let dir = test_dir("roundtrip");
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        dir.clone(),
    );

    let stamp = Uuid::now_v7().simple().to_string()[..8].to_owned();
    let alice = auth::create_user(
        &pool,
        &format!("atal{stamp}"),
        &format!("atal{stamp}@example.com"),
        "AttAlice",
        "pw-alice-att",
    )
    .await
    .expect("register alice");
    let bob = auth::create_user(
        &pool,
        &format!("atbo{stamp}"),
        &format!("atbo{stamp}@example.com"),
        "AttBob",
        "pw-bob-att",
    )
    .await
    .expect("register bob");
    let mallory = auth::create_user(
        &pool,
        &format!("atma{stamp}"),
        &format!("atma{stamp}@example.com"),
        "AttMallory",
        "pw-mallory-att",
    )
    .await
    .expect("register mallory");
    let alice_token = auth::login(&pool, &alice.handle, "pw-alice-att")
        .await
        .expect("login alice")
        .token;
    let bob_token = auth::login(&pool, &bob.handle, "pw-bob-att")
        .await
        .expect("login bob")
        .token;
    let mallory_token = auth::login(&pool, &mallory.handle, "pw-mallory-att")
        .await
        .expect("login mallory")
        .token;
    let dm = messaging::find_or_create_dm(&pool, alice.id, bob.id)
        .await
        .expect("open dm");

    upload_download_round_trip(&state, &dm.id, &alice_token, &bob_token, &mallory_token).await;

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// Uploads `png` as Alice, asserts Bob's byte-identical download, then
/// asserts Mallory (non-member) sees the same 404 as a missing id.
async fn upload_download_round_trip(
    state: &AppState,
    dm_id: &Uuid,
    alice_token: &str,
    bob_token: &str,
    mallory_token: &str,
) {
    // Minimal PNG bytes (signature + IHDR); stored and served verbatim.
    let png: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52,
    ];
    let (status, body) = post_bytes(
        build_router(state.clone()),
        &format!("/v1/conversations/{dm_id}/attachments"),
        alice_token,
        "photo.png",
        "image/png",
        &png,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "upload must succeed");
    let view: serde_json::Value = serde_json::from_slice(&body).expect("upload json");
    let id = view["id"].as_str().expect("upload id").to_owned();
    assert_eq!(
        view["size_bytes"],
        i64::try_from(png.len()).expect("fixture fits in i64")
    );
    let expected_sha = STANDARD.encode(Sha256::digest(&png));
    assert_eq!(view["sha256"], expected_sha);

    // Bob downloads the exact bytes with the stored content type.
    let (status, bytes, content_type) = get_bytes(
        build_router(state.clone()),
        &format!("/v1/attachments/{id}"),
        bob_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, png, "bytes must round-trip identical");
    assert_eq!(content_type, "image/png");

    // Mallory (non-member) gets the same 404 as a missing id: no oracle.
    let (status, _, _) = get_bytes(
        build_router(state.clone()),
        &format!("/v1/attachments/{id}"),
        mallory_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let missing = Uuid::now_v7();
    let (status, _, _) = get_bytes(
        build_router(state.clone()),
        &format!("/v1/attachments/{missing}"),
        bob_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Oversize and off-allowlist uploads are rejected with 413 / 415.
#[tokio::test]
async fn attachment_caps_and_allowlist() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: attachment_caps_and_allowlist (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let dir = test_dir("caps");
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        dir.clone(),
    );

    let stamp = Uuid::now_v7().simple().to_string()[..8].to_owned();
    let alice = auth::create_user(
        &pool,
        &format!("cpal{stamp}"),
        &format!("cpal{stamp}@example.com"),
        "CapAlice",
        "pw-alice-cap",
    )
    .await
    .expect("register alice");
    let bob = auth::create_user(
        &pool,
        &format!("cpbo{stamp}"),
        &format!("cpbo{stamp}@example.com"),
        "CapBob",
        "pw-bob-cap",
    )
    .await
    .expect("register bob");
    let alice_token = auth::login(&pool, &alice.handle, "pw-alice-cap")
        .await
        .expect("login alice")
        .token;
    let dm = messaging::find_or_create_dm(&pool, alice.id, bob.id)
        .await
        .expect("open dm");

    // 10 MiB + 1 byte → 413.
    let big = vec![0x41u8; 10 * 1024 * 1024 + 1];
    let (status, _) = post_bytes(
        build_router(state.clone()),
        &format!("/v1/conversations/{}/attachments", dm.id),
        &alice_token,
        "big.bin",
        "image/png",
        &big,
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // Off-allowlist MIME → 415.
    let (status, _) = post_bytes(
        build_router(state.clone()),
        &format!("/v1/conversations/{}/attachments", dm.id),
        &alice_token,
        "evil.sh",
        "application/x-sh",
        b"#!/bin/sh",
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // GIF (the owner's explicit ask) is allowed.
    let gif: Vec<u8> = b"GIF89a".to_vec();
    let (status, _) = post_bytes(
        build_router(state.clone()),
        &format!("/v1/conversations/{}/attachments", dm.id),
        &alice_token,
        "dance.gif",
        "image/gif",
        &gif,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// Concurrent uploads from both members get unique ids and intact bytes.
#[tokio::test]
async fn attachment_concurrent_uploads_unique() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: attachment_concurrent_uploads_unique (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let dir = test_dir("concurrent");
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        dir.clone(),
    );

    let stamp = Uuid::now_v7().simple().to_string()[..8].to_owned();
    let alice = auth::create_user(
        &pool,
        &format!("cnal{stamp}"),
        &format!("cnal{stamp}@example.com"),
        "ConAlice",
        "pw-alice-con",
    )
    .await
    .expect("register alice");
    let bob = auth::create_user(
        &pool,
        &format!("cnbo{stamp}"),
        &format!("cnbo{stamp}@example.com"),
        "ConBob",
        "pw-bob-con",
    )
    .await
    .expect("register bob");
    let alice_token = auth::login(&pool, &alice.handle, "pw-alice-con")
        .await
        .expect("login alice")
        .token;
    let bob_token = auth::login(&pool, &bob.handle, "pw-bob-con")
        .await
        .expect("login bob")
        .token;
    let dm = messaging::find_or_create_dm(&pool, alice.id, bob.id)
        .await
        .expect("open dm");

    let mut set = tokio::task::JoinSet::new();
    for i in 0..8u8 {
        let state = state.clone();
        let token = if i % 2 == 0 {
            alice_token.clone()
        } else {
            bob_token.clone()
        };
        let dm_id = dm.id;
        let bytes = vec![i; 64];
        set.spawn(async move {
            let app = build_router(state);
            let uri = format!("/v1/conversations/{dm_id}/attachments");
            post_bytes(app, &uri, &token, &format!("f{i}.png"), "image/png", &bytes).await
        });
    }
    let mut ids = std::collections::HashSet::new();
    while let Some(outcome) = set.join_next().await {
        let (status, body) = outcome.expect("upload task");
        assert_eq!(status, StatusCode::CREATED);
        let view: serde_json::Value = serde_json::from_slice(&body).expect("upload json");
        assert!(ids.insert(view["id"].as_str().expect("id").to_owned()));
    }
    assert_eq!(ids.len(), 8, "every concurrent upload gets a unique id");

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}
