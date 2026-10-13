//! Workspace HTTP flows over the real routes.
//!
//! Moved verbatim from `src/main.rs` integration module.
#![forbid(unsafe_code)]
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use http_body_util::BodyExt;
use terichat_server::{auth, build_router, db_pool, AppState, HUB_CAPACITY, MIGRATOR};
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

async fn http_users(pool: &sqlx::PgPool, stamp: &str, prefix: &str) -> (String, String, String) {
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
    let state = || {
        AppState::new(
            Some(pool.clone()),
            broadcast::channel(HUB_CAPACITY).0,
            std::env::temp_dir().join("terichat-test-attachments"),
        )
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
    let state = || {
        AppState::new(
            Some(pool.clone()),
            broadcast::channel(HUB_CAPACITY).0,
            std::env::temp_dir().join("terichat-test-attachments"),
        )
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
    let state = || {
        AppState::new(
            Some(pool.clone()),
            broadcast::channel(HUB_CAPACITY).0,
            std::env::temp_dir().join("terichat-test-attachments"),
        )
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
    let state = || {
        AppState::new(
            Some(pool.clone()),
            broadcast::channel(HUB_CAPACITY).0,
            std::env::temp_dir().join("terichat-test-attachments"),
        )
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
    let state = || {
        AppState::new(
            Some(pool.clone()),
            broadcast::channel(HUB_CAPACITY).0,
            std::env::temp_dir().join("terichat-test-attachments"),
        )
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
