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
use terichat_server::{
    attachments, auth, build_router, db_pool, messaging, workspaces, AppState, HUB_CAPACITY,
    MIGRATOR,
};
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
    raw.replace('%', "%25")
        .replace('+', "%2B")
        .replace(';', "%3B")
        .replace('=', "%3D")
        .replace(' ', "%20")
        .replace('/', "%2F")
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

/// Two synthetic members of a fresh DM plus attachment storage in `dir`.
async fn dm_fixture(
    pool: &sqlx::PgPool,
    tag: &str,
) -> (
    Uuid,
    Uuid,
    Uuid,
    attachments::FilesystemAttachmentStorage,
    std::path::PathBuf,
) {
    // The random tail, not the timestamp prefix: reruns must not collide.
    let stamp = Uuid::now_v7().simple().to_string()[24..].to_owned();
    let mut ids = Vec::new();
    for who in ["a", "b"] {
        let handle = format!("{tag}{who}{stamp}");
        let user = auth::create_user(
            pool,
            &handle,
            &format!("{handle}@example.com"),
            "Synthetic",
            "pw-synthetic-att",
        )
        .await
        .expect("register synthetic user");
        ids.push(user.id);
    }
    let dm = messaging::find_or_create_dm(pool, ids[0], ids[1])
        .await
        .expect("open dm");
    let dir = test_dir(tag);
    let storage = attachments::FilesystemAttachmentStorage::new(dir.clone());
    (ids[0], ids[1], dm.id, storage, dir)
}

/// Begin a kick of `user_id` from `conversation_id` without committing it.
async fn pending_kick(
    pool: &sqlx::PgPool,
    conversation_id: Uuid,
    user_id: Uuid,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut kick = pool.begin().await.expect("begin kick");
    sqlx::query(
        "DELETE FROM conversation_participants WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(conversation_id)
    .bind(user_id)
    .execute(&mut *kick)
    .await
    .expect("delete participant");
    kick
}

/// An upload racing a kick waits for it and is refused once it commits:
/// no row and no stored bytes survive (issue #54 item 1).
#[tokio::test]
async fn attachment_upload_serializes_with_kick() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: attachment_upload_serializes_with_kick (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let (_alice, bob, dm, storage, dir) = dm_fixture(&pool, "upk").await;

    let kick = pending_kick(&pool, dm, bob).await;
    let upload = {
        let (pool, storage) = (pool.clone(), storage.clone());
        tokio::spawn(async move {
            attachments::upload_attachment(
                &pool,
                &storage,
                bob,
                dm,
                "late.png",
                "image/png",
                b"late",
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !upload.is_finished(),
        "the upload must wait for the in-flight kick, not slip past it"
    );
    kick.commit().await.expect("commit kick");
    let outcome = upload.await.expect("upload task");
    assert!(
        matches!(outcome, Err(attachments::AttachmentError::NotFound)),
        "upload after the kick is refused: {outcome:?}"
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE conversation_id = $1")
            .bind(dm)
            .fetch_one(&pool)
            .await
            .expect("count rows");
    assert_eq!(rows, 0);
    let files = std::fs::read_dir(&dir).map_or(0, Iterator::count);
    assert_eq!(files, 0, "refused upload leaves no bytes behind");

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// A download racing a kick waits for it and never returns the bytes after
/// the revocation commits.
#[tokio::test]
async fn attachment_download_serializes_with_kick() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: attachment_download_serializes_with_kick (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let (alice, bob, dm, storage, dir) = dm_fixture(&pool, "dnk").await;
    let stored =
        attachments::upload_attachment(&pool, &storage, alice, dm, "a.png", "image/png", b"png")
            .await
            .expect("alice uploads");

    let kick = pending_kick(&pool, dm, bob).await;
    let download = {
        let (pool, storage) = (pool.clone(), storage.clone());
        tokio::spawn(async move {
            attachments::download_attachment(&pool, &storage, bob, stored.id)
                .await
                .map(|(_, bytes)| bytes)
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !download.is_finished(),
        "the download must wait for the in-flight kick, not slip past it"
    );
    kick.commit().await.expect("commit kick");
    let outcome = download.await.expect("download task");
    assert!(
        matches!(outcome, Err(attachments::AttachmentError::NotFound)),
        "download after the kick is refused: {outcome:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// Deleting a conversation cascades its attachment rows; the trigger queues
/// their bytes and the sweeper deletes them (issue #54 item 3).
#[tokio::test]
async fn attachment_blobs_are_collected_after_cascade() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: attachment_blobs_are_collected_after_cascade (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let (alice, _bob, dm, storage, dir) = dm_fixture(&pool, "gcc").await;
    let stored = attachments::upload_attachment(
        &pool,
        &storage,
        alice,
        dm,
        "gone.zip",
        "application/zip",
        b"zip",
    )
    .await
    .expect("alice uploads");
    let path = dir.join(stored.id.simple().to_string());
    assert!(path.exists(), "bytes stored on upload");

    sqlx::query("DELETE FROM conversations WHERE id = $1")
        .bind(dm)
        .execute(&pool)
        .await
        .expect("delete conversation");
    let queued: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM attachment_blob_gc WHERE attachment_id = $1)",
    )
    .bind(stored.id)
    .fetch_one(&pool)
    .await
    .expect("read tombstone");
    assert!(queued, "the cascade queues the bytes for collection");
    assert!(path.exists(), "bytes stay until the sweeper runs");

    // Other tests may queue tombstones too; sweep until ours is cleared.
    for _ in 0..10 {
        attachments::sweep_deleted_blobs(&pool, &storage, 1000)
            .await
            .expect("sweep");
        if !path.exists() {
            break;
        }
    }
    assert!(!path.exists(), "the sweeper deletes the orphaned bytes");
    let queued: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM attachment_blob_gc WHERE attachment_id = $1)",
    )
    .bind(stored.id)
    .fetch_one(&pool)
    .await
    .expect("read tombstone");
    assert!(!queued, "a collected tombstone is removed");

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// A `send_messages` denial committed while the body is still transferring
/// refuses the upload: the channel gate runs again after the transfer.
#[tokio::test]
async fn attachment_upload_rechecks_channel_send_after_transfer() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!(
            "SKIPPED: attachment_upload_rechecks_channel_send_after_transfer (DATABASE_URL unset)"
        );
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let dir = test_dir("chanrecheck");
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        dir.clone(),
    );
    let stamp = Uuid::now_v7().simple().to_string()[24..].to_owned();
    let mut users = Vec::new();
    for who in ["own", "mem"] {
        let handle = format!("crk{who}{stamp}");
        let user = auth::create_user(
            &pool,
            &handle,
            &format!("{handle}@example.com"),
            "Synthetic",
            "pw-synthetic-att",
        )
        .await
        .expect("register synthetic user");
        users.push(user);
    }
    let (owner, member) = (users[0].id, users[1].id);
    let token = auth::login(&pool, &users[1].handle, "pw-synthetic-att")
        .await
        .expect("login member")
        .token;
    let workspace = workspaces::create_workspace(&pool, owner, "recheck")
        .await
        .expect("create workspace");
    workspaces::add_member(&pool, owner, workspace.id, member, workspaces::Role::Member)
        .await
        .expect("add member");
    let channel = workspaces::create_channel(&pool, owner, workspace.id, "general")
        .await
        .expect("create channel");

    // Stream the body in two parts so the denial lands mid-transfer.
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|chunk| (Ok::<_, std::io::Error>(chunk), rx))
    });
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/v1/conversations/{}/attachments?filename=late.png&mime=image%2Fpng",
            channel.conversation_id
        ))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap();
    let app = build_router(state);
    let upload = tokio::spawn(async move { app.oneshot(request).await.unwrap().status() });
    tx.send(b"first half".to_vec())
        .await
        .expect("send first chunk");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    workspaces::set_override(
        &pool,
        owner,
        channel.id,
        "member",
        &member.to_string(),
        workspaces::Permission::SendMessages,
        false,
    )
    .await
    .expect("deny member");
    tx.send(b"second half".to_vec())
        .await
        .expect("send second chunk");
    drop(tx);
    assert_eq!(upload.await.expect("upload task"), StatusCode::NOT_FOUND);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE conversation_id = $1")
            .bind(channel.conversation_id)
            .fetch_one(&pool)
            .await
            .expect("count rows");
    assert_eq!(rows, 0, "a revoked sender stores nothing");

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// A download holds a slot for as long as its response body is alive, so
/// unread responses cannot accumulate blobs past the slot count.
#[tokio::test]
async fn attachment_download_holds_a_slot_until_the_body_drops() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!(
            "SKIPPED: attachment_download_holds_a_slot_until_the_body_drops (DATABASE_URL unset)"
        );
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let dir = test_dir("dlslot");
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        dir.clone(),
    );
    let stamp = Uuid::now_v7().simple().to_string()[24..].to_owned();
    let mut users = Vec::new();
    for who in ["a", "b"] {
        let handle = format!("dls{who}{stamp}");
        users.push(
            auth::create_user(
                &pool,
                &handle,
                &format!("{handle}@example.com"),
                "Synthetic",
                "pw-synthetic-att",
            )
            .await
            .expect("register synthetic user"),
        );
    }
    let token = auth::login(&pool, &users[0].handle, "pw-synthetic-att")
        .await
        .expect("login")
        .token;
    let dm = messaging::find_or_create_dm(&pool, users[0].id, users[1].id)
        .await
        .expect("open dm");
    let payload = vec![7u8; 200 * 1024];
    let (status, body) = post_bytes(
        build_router(state.clone()),
        &format!("/v1/conversations/{}/attachments", dm.id),
        &token,
        "big.png",
        "image/png",
        &payload,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let full = terichat_server::state::MAX_CONCURRENT_DOWNLOADS;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/v1/attachments/{id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(state.download_slots.available_permits(), full - 1);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), payload.as_slice());
    assert_eq!(state.download_slots.available_permits(), full);

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// MIME parameters cannot disguise an SVG: the stored type is the essence,
/// so the download is still forced to `attachment` and sandboxed.
#[tokio::test]
async fn svg_with_parameters_still_downloads_as_attachment() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!(
            "SKIPPED: svg_with_parameters_still_downloads_as_attachment (DATABASE_URL unset)"
        );
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let dir = test_dir("svgparam");
    let state = AppState::new(
        Some(pool.clone()),
        broadcast::channel(HUB_CAPACITY).0,
        dir.clone(),
    );
    let stamp = Uuid::now_v7().simple().to_string()[24..].to_owned();
    let mut users = Vec::new();
    for who in ["a", "b"] {
        let handle = format!("svg{who}{stamp}");
        users.push(
            auth::create_user(
                &pool,
                &handle,
                &format!("{handle}@example.com"),
                "Synthetic",
                "pw-synthetic-att",
            )
            .await
            .expect("register synthetic user"),
        );
    }
    let token = auth::login(&pool, &users[0].handle, "pw-synthetic-att")
        .await
        .expect("login")
        .token;
    let dm = messaging::find_or_create_dm(&pool, users[0].id, users[1].id)
        .await
        .expect("open dm");
    let (status, body) = post_bytes(
        build_router(state.clone()),
        &format!("/v1/conversations/{}/attachments", dm.id),
        &token,
        "x.svg",
        "image/svg+xml; charset=utf-8",
        b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(created["mime"], "image/svg+xml");

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/attachments/{}",
                    created["id"].as_str().unwrap()
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    assert!(disposition.starts_with("attachment;"), "{disposition}");
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("sandbox"), "{csp}");

    let _ = std::fs::remove_dir_all(&dir);
    pool.close().await;
}

/// Storage that deletes only `ok` and fails every other id.
#[derive(Debug)]
struct OnlyDeletes {
    ok: Uuid,
}

impl attachments::AttachmentStorage for OnlyDeletes {
    async fn put(&self, _id: Uuid, _bytes: &[u8]) -> Result<(), String> {
        Err("read-only".to_owned())
    }
    async fn get(&self, _id: Uuid) -> Result<Vec<u8>, String> {
        Err("empty".to_owned())
    }
    async fn delete(&self, id: Uuid) -> Result<(), String> {
        if id == self.ok {
            Ok(())
        } else {
            Err("poisoned".to_owned())
        }
    }
}

/// A tombstone whose delete keeps failing rotates behind untried work
/// instead of being re-picked by every pass.
#[tokio::test]
async fn failing_tombstones_do_not_starve_the_gc_queue() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: failing_tombstones_do_not_starve_the_gc_queue (DATABASE_URL unset)");
        return;
    };
    let pool = db_pool(&url).await.expect("connect test database");
    MIGRATOR.run(&pool).await.expect("apply migrations");
    let (poisoned, good) = (Uuid::now_v7(), Uuid::now_v7());
    // The poisoned tombstone is the oldest in the queue.
    sqlx::query(
        "INSERT INTO attachment_blob_gc (attachment_id, enqueued_at)
         VALUES ($1, '2000-01-01'), ($2, now())",
    )
    .bind(poisoned)
    .bind(good)
    .execute(&pool)
    .await
    .expect("queue tombstones");
    let storage = OnlyDeletes { ok: good };
    let queued = |id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM attachment_blob_gc WHERE attachment_id = $1)",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("read tombstone")
        }
    };
    // One tombstone per pass; other tests' rows may share the queue.
    for _ in 0..200 {
        attachments::sweep_deleted_blobs(&pool, &storage, 1)
            .await
            .expect("sweep");
        if !queued(good).await {
            break;
        }
    }
    assert!(!queued(good).await, "the deletable tombstone is reached");

    sqlx::query("DELETE FROM attachment_blob_gc WHERE attachment_id = $1")
        .bind(poisoned)
        .execute(&pool)
        .await
        .expect("cleanup");
    pool.close().await;
}
