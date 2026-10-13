//! `PostgreSQL` regressions for private read markers: the upgrade backfill,
//! the HTTP contract, and channel gating parity with history. No skip: an
//! unset `DATABASE_URL` is NOT RUN, never a pass.
#![forbid(unsafe_code)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use sqlx::{migrate::Migrator, postgres::PgPoolOptions, PgPool};
use terichat_server::{build_router, messaging, workspaces, AppState, HUB_CAPACITY, MIGRATOR};
use tower::ServiceExt;
use uuid::Uuid;

const READ_MARKERS: &str = "20260926090000_read_markers.sql";

/// One randomly named schema per test; migrations and data stay inside it.
struct Fixture {
    pool: PgPool,
    admin: PgPool,
    schema: String,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("DATABASE_URL")
            .expect("read markers require synthetic PostgreSQL; unset is NOT RUN");
        let schema = format!("read_markers_{}", Uuid::now_v7().simple());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let search_path = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .after_connect(move |connection, _| {
                let search_path = search_path.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('search_path', $1, false)")
                        .bind(search_path)
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        Self {
            pool,
            admin,
            schema,
        }
    }

    async fn user(&self) -> (Uuid, String) {
        let id = Uuid::now_v7();
        let handle = id.simple().to_string();
        sqlx::query("INSERT INTO users (id, handle, email, display_name) VALUES ($1, $2, $2 || '@example.invalid', 'Synthetic')")
            .bind(id).bind(&handle).execute(&self.pool).await.unwrap();
        let token = format!("synthetic-{id}");
        sqlx::query("INSERT INTO sessions (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, now() + interval '1 hour')")
            .bind(Uuid::now_v7()).bind(id).bind(Sha256::digest(token.as_bytes()).to_vec())
            .execute(&self.pool).await.unwrap();
        (id, token)
    }

    fn router(&self) -> Router {
        let (hub, _) = tokio::sync::broadcast::channel(HUB_CAPACITY);
        build_router(AppState {
            pool: Some(self.pool.clone()),
            hub,
        })
    }

    async fn close(self) {
        self.pool.close().await;
        // Only the randomly named synthetic schema created by this fixture.
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}

async fn call(
    app: Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |json| Body::from(json.to_string())))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn upgrade_backfills_existing_members_as_read() {
    let f = Fixture::new().await;
    // Apply every migration before read markers from a copy of the tree, as an
    // already-deployed database would have, then seed history.
    let before =
        std::env::temp_dir().join(format!("read-markers-before-{}", Uuid::now_v7().simple()));
    std::fs::create_dir_all(&before).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    for entry in std::fs::read_dir(&source).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|name| name != READ_MARKERS) {
            std::fs::copy(&path, before.join(path.file_name().unwrap())).unwrap();
        }
    }
    assert!(
        source.join(READ_MARKERS).exists(),
        "migration under test is present"
    );
    Migrator::new(before.as_path())
        .await
        .unwrap()
        .run(&f.pool)
        .await
        .unwrap();
    std::fs::remove_dir_all(&before).unwrap();

    let (a, _) = f.user().await;
    let (b, _) = f.user().await;
    let busy = Uuid::now_v7();
    let quiet = Uuid::now_v7();
    for (id, next_seq) in [(busy, 8_i64), (quiet, 1)] {
        sqlx::query("INSERT INTO conversations (id, kind, created_by, next_seq) VALUES ($1, 'group', $2, $3)")
            .bind(id).bind(a).bind(next_seq).execute(&f.pool).await.unwrap();
        for member in [a, b] {
            sqlx::query(
                "INSERT INTO conversation_participants (conversation_id, user_id) VALUES ($1, $2)",
            )
            .bind(id)
            .bind(member)
            .execute(&f.pool)
            .await
            .unwrap();
        }
    }

    // The real, embedded migration set: earlier checksums must still match.
    MIGRATOR.run(&f.pool).await.unwrap();
    let markers: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT conversation_id, last_read_seq FROM conversation_participants ORDER BY conversation_id, user_id",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    for (conversation, marker) in markers {
        let expected = if conversation == busy { 7 } else { 0 };
        assert_eq!(
            marker, expected,
            "existing members start read up to the last assigned seq"
        );
    }
    // Memberships created after the upgrade start unread from the beginning.
    let (c, _) = f.user().await;
    sqlx::query("INSERT INTO conversation_participants (conversation_id, user_id) VALUES ($1, $2)")
        .bind(busy)
        .bind(c)
        .execute(&f.pool)
        .await
        .unwrap();
    let fresh: i64 = sqlx::query_scalar("SELECT last_read_seq FROM conversation_participants WHERE conversation_id = $1 AND user_id = $2")
        .bind(busy).bind(c).fetch_one(&f.pool).await.unwrap();
    assert_eq!(fresh, 0);
    let rejected =
        sqlx::query("UPDATE conversation_participants SET last_read_seq = -1 WHERE user_id = $1")
            .bind(c)
            .execute(&f.pool)
            .await;
    assert!(
        rejected.is_err(),
        "negative markers violate the CHECK constraint"
    );
    f.close().await;
}

#[tokio::test]
async fn http_read_marker_contract_and_privacy() {
    let f = Fixture::new().await;
    MIGRATOR.run(&f.pool).await.unwrap();
    let (a, a_token) = f.user().await;
    let (b, b_token) = f.user().await;
    let (_, stranger) = f.user().await;
    let dm = messaging::find_or_create_dm(&f.pool, a, b).await.unwrap();
    for _ in 0..2 {
        messaging::send_message(&f.pool, b, dm.id, Uuid::now_v7(), b"opaque", None)
            .await
            .unwrap();
    }

    let uri = format!("/v1/conversations/{}/read", dm.id);
    let (status, body) = call(
        f.router(),
        "POST",
        &uri,
        &a_token,
        Some(serde_json::json!({"seq": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        serde_json::json!({"conversation_id": dm.id, "last_read_seq": 1})
    );
    let (_, body) = call(
        f.router(),
        "POST",
        &uri,
        &a_token,
        Some(serde_json::json!({"seq": 50})),
    )
    .await;
    assert_eq!(body["last_read_seq"], 2, "clamped to the last message");

    // Each caller's list carries only their own marker.
    let marker = |rows: &serde_json::Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == dm.id.to_string())
            .unwrap()["last_read_seq"]
            .clone()
    };
    let (_, a_rows) = call(f.router(), "GET", "/v1/conversations", &a_token, None).await;
    let (_, b_rows) = call(f.router(), "GET", "/v1/conversations", &b_token, None).await;
    assert_eq!(marker(&a_rows), 2);
    assert_eq!(marker(&b_rows), 2, "B's marker comes from B's own sends");

    let (status, _) = call(
        f.router(),
        "POST",
        &uri,
        &stranger,
        Some(serde_json::json!({"seq": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        f.router(),
        "POST",
        &format!("/v1/conversations/{}/read", Uuid::now_v7()),
        &a_token,
        Some(serde_json::json!({"seq": 1})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "unknown and foreign conversations look the same"
    );
    f.close().await;
}

#[tokio::test]
async fn channel_read_marker_uses_the_history_gate() {
    let f = Fixture::new().await;
    MIGRATOR.run(&f.pool).await.unwrap();
    let (owner, owner_token) = f.user().await;
    let (_, stranger) = f.user().await;
    let workspace = workspaces::create_workspace(&f.pool, owner, "Read gate")
        .await
        .unwrap();
    let channel = workspaces::create_channel(&f.pool, owner, workspace.id, "reads")
        .await
        .unwrap();
    messaging::send_message(
        &f.pool,
        owner,
        channel.conversation_id,
        Uuid::now_v7(),
        b"opaque",
        None,
    )
    .await
    .unwrap();

    let read_uri = format!("/v1/conversations/{}/read", channel.conversation_id);
    let history_uri = format!("/v1/messages?conversation_id={}", channel.conversation_id);
    let (history, _) = call(f.router(), "GET", &history_uri, &stranger, None).await;
    let (read, _) = call(
        f.router(),
        "POST",
        &read_uri,
        &stranger,
        Some(serde_json::json!({"seq": 1})),
    )
    .await;
    assert!(history.is_client_error());
    assert_eq!(
        read, history,
        "a stranger learns nothing new from the read endpoint"
    );

    let (status, body) = call(
        f.router(),
        "POST",
        &read_uri,
        &owner_token,
        Some(serde_json::json!({"seq": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["last_read_seq"], 1);
    f.close().await;
}

/// Same rules as the endpoint tests, at the domain layer. Lives in an isolated
/// schema: sends here must never land in the shared outbox other tests drain.
#[tokio::test]
async fn read_markers_are_private_monotonic_and_clamped() {
    let f = Fixture::new().await;
    MIGRATOR.run(&f.pool).await.unwrap();
    let (ada, _) = f.user().await;
    let (bob, _) = f.user().await;
    let (eve, _) = f.user().await;
    let dm = messaging::find_or_create_dm(&f.pool, ada, bob)
        .await
        .unwrap();
    let marker = |user: Uuid| {
        let pool = f.pool.clone();
        async move {
            messaging::list_conversations(&pool, user)
                .await
                .unwrap()
                .into_iter()
                .find(|row| row.id == dm.id)
                .map(|row| (row.last_read_seq, row.last_seq))
        }
    };
    let send =
        |user: Uuid| messaging::send_message(&f.pool, user, dm.id, Uuid::now_v7(), b"opaque", None);

    for _ in 0..3 {
        send(bob).await.unwrap();
    }
    // Sending marks your own messages read; the recipient starts at 0.
    assert_eq!(marker(bob).await, Some((3, Some(3))));
    assert_eq!(marker(ada).await, Some((0, Some(3))));

    assert_eq!(
        messaging::mark_read(&f.pool, ada, dm.id, 2).await.unwrap(),
        2
    );
    assert_eq!(
        messaging::mark_read(&f.pool, ada, dm.id, 1).await.unwrap(),
        2,
        "never rewinds"
    );
    assert_eq!(
        messaging::mark_read(&f.pool, ada, dm.id, 99).await.unwrap(),
        3,
        "clamped to last seq"
    );
    assert_eq!(
        messaging::mark_read(&f.pool, ada, dm.id, -5).await.unwrap(),
        3
    );

    // Ada's reads never touch Bob's marker, and Ada's send leaves Bob unread.
    send(ada).await.unwrap();
    assert_eq!(marker(ada).await, Some((4, Some(4))));
    assert_eq!(marker(bob).await, Some((3, Some(4))));

    // Bob replies before reading Ada's seq 4: his send must not step over it,
    // so it stays unread for him (review: concurrent unseen message).
    send(bob).await.unwrap();
    assert_eq!(marker(bob).await, Some((3, Some(5))));
    assert_eq!(
        messaging::mark_read(&f.pool, bob, dm.id, 5).await.unwrap(),
        5
    );

    assert!(matches!(
        messaging::mark_read(&f.pool, eve, dm.id, 4).await,
        Err(messaging::MessagingError::NotMember)
    ));
    assert!(matches!(
        messaging::mark_read(&f.pool, ada, Uuid::now_v7(), 1).await,
        Err(messaging::MessagingError::NotMember)
    ));
    f.close().await;
}
