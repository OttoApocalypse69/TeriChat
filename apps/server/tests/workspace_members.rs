//! Workspace member roster routes over the real router.
//!
//! Moved verbatim from `src/routes.rs` member tests: behavior-preserving,
//! no API changes.
#![forbid(unsafe_code)]
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use terichat_server::{auth, build_router, workspaces, AppState, HUB_CAPACITY, MIGRATOR};
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    pool: sqlx::PgPool,
    app: Router,
    workspace: Uuid,
    owner: Uuid,
    owner_token: String,
    guest: Uuid,
    guest_handle: String,
    guest_token: String,
}

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("SKIPPED: workspace member route tests (DATABASE_URL unset)");
        return None;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("member route database");
    MIGRATOR.run(&pool).await.expect("migrations");
    let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let mut users = Vec::new();
    for name in ["mhttpowner", "mhttpguest"] {
        let handle = format!("{name}{stamp}");
        let user = auth::create_user(
            &pool,
            &handle,
            &format!("{handle}@example.invalid"),
            name,
            "synthetic-route-password",
        )
        .await
        .expect("synthetic account");
        let token = auth::login(&pool, &handle, "synthetic-route-password")
            .await
            .expect("login")
            .token;
        users.push((user, token));
    }
    let (guest, guest_token) = users.pop().expect("guest");
    let (owner, owner_token) = users.pop().expect("owner");
    let workspace = workspaces::create_workspace(&pool, owner.id, "HTTP roster")
        .await
        .expect("workspace")
        .id;
    let (hub, _) = tokio::sync::broadcast::channel(HUB_CAPACITY);
    let app = build_router(AppState::new(
        Some(pool.clone()),
        hub,
        std::env::temp_dir().join("terichat-test-attachments"),
    ));
    Some(Fixture {
        pool,
        app,
        workspace,
        owner: owner.id,
        owner_token,
        guest: guest.id,
        guest_handle: guest.handle,
        guest_token,
    })
}

async fn request(
    f: &Fixture,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = body.map_or_else(Body::empty, |json| Body::from(json.to_string()));
    let response = f
        .app
        .clone()
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(body)
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    (
        status,
        response
            .into_body()
            .collect()
            .await
            .expect("response bytes")
            .to_bytes()
            .to_vec(),
    )
}

#[tokio::test]
async fn members_route_pagination_public_fields_and_post_preserved() {
    let Some(f) = fixture().await else { return };
    let uri = format!("/v1/workspaces/{}/members", f.workspace);
    let (status, _) = request(
        &f,
        "POST",
        &uri,
        Some(&f.owner_token),
        Some(serde_json::json!({"user_handle": f.guest_handle, "role": "guest"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, bytes) = request(
        &f,
        "GET",
        &format!("{uri}?limit=1"),
        Some(&f.guest_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(first.as_object().expect("page object").len(), 2);
    assert_eq!(first["members"].as_array().expect("members").len(), 1);
    let first_id = first["members"][0]["user_id"].as_str().expect("first id");
    assert_eq!(first["next_cursor"], first_id);
    let (status, bytes) = request(
        &f,
        "GET",
        &format!("{uri}?after={first_id}&limit=1"),
        Some(&f.guest_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let last: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert!(last["next_cursor"].is_null());
    assert_eq!(last["members"].as_array().expect("members").len(), 1);
    assert!(first_id < last["members"][0]["user_id"].as_str().expect("last id"));
    for page in [&first, &last] {
        let profile = page["members"][0].as_object().expect("profile");
        let mut keys: Vec<&str> = profile.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["display_name", "handle", "joined_at", "role", "user_id"]
        );
        DateTime::parse_from_rfc3339(profile["joined_at"].as_str().expect("join time"))
            .expect("valid timestamp");
    }
    let (status, bytes) = request(&f, "GET", &uri, Some(&f.owner_token), None).await;
    assert_eq!(status, StatusCode::OK);
    let all: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(all["members"].as_array().expect("members").len(), 2);
    assert!(all["next_cursor"].is_null());
}

#[tokio::test]
async fn members_route_rejects_outsiders_revoked_and_bad_queries() {
    let Some(f) = fixture().await else { return };
    let uri = format!("/v1/workspaces/{}/members", f.workspace);
    assert_eq!(
        request(&f, "GET", &uri, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let outside = request(&f, "GET", &uri, Some(&f.guest_token), None).await;
    let missing = request(
        &f,
        "GET",
        &format!("/v1/workspaces/{}/members", Uuid::now_v7()),
        Some(&f.guest_token),
        None,
    )
    .await;
    assert_eq!(outside.0, StatusCode::FORBIDDEN);
    assert_eq!(outside, missing, "no workspace existence oracle");
    for query in ["after=not-a-uuid", "limit=0", "limit=-1", "limit=nope"] {
        assert_eq!(
            request(
                &f,
                "GET",
                &format!("{uri}?{query}"),
                Some(&f.owner_token),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    workspaces::add_member(
        &f.pool,
        f.owner,
        f.workspace,
        f.guest,
        workspaces::Role::Guest,
    )
    .await
    .expect("join");
    workspaces::leave(&f.pool, f.guest, f.workspace)
        .await
        .expect("leave");
    assert_eq!(
        request(&f, "GET", &uri, Some(&f.guest_token), None).await,
        outside
    );
    workspaces::add_member(
        &f.pool,
        f.owner,
        f.workspace,
        f.guest,
        workspaces::Role::Guest,
    )
    .await
    .expect("rejoin");
    workspaces::ban_member(&f.pool, f.owner, f.workspace, f.guest, "synthetic ban")
        .await
        .expect("ban");
    let banned = request(&f, "GET", &uri, Some(&f.guest_token), None).await;
    assert_eq!(banned.0, StatusCode::FORBIDDEN);
    let json: serde_json::Value = serde_json::from_slice(&banned.1).expect("error json");
    assert_eq!(json["error"]["message"], "banned from this workspace");
    assert!(json.get("members").is_none());
}
