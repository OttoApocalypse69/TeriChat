//! Mandatory `PostgreSQL` regressions for S4-DM-ATOMIC. No skip without a DB.
#![forbid(unsafe_code)]

use axum::{body::Body, http::Request, Router};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;
use terichat_server::{build_router, messaging, AppState, HUB_CAPACITY, MIGRATOR};
use tokio::{task::JoinSet, time::timeout};
use tower::ServiceExt;
use uuid::Uuid;

const DEADLINE: Duration = Duration::from_secs(15);

struct Fixture {
    observer: PgPool,
    pool: PgPool,
    schema: String,
}

async fn scoped_pool(url: &str, schema: &str, name: &str, size: u32) -> PgPool {
    let schema = schema.to_owned();
    let name = name.to_owned();
    PgPoolOptions::new()
        .max_connections(size)
        .acquire_timeout(DEADLINE)
        .after_connect(move |connection, _| {
            let schema = schema.clone();
            let name = name.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('search_path', $1, false), set_config('application_name', $2, false), set_config('statement_timeout', '15000', false)")
                    .bind(schema).bind(name).execute(connection).await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .expect("synthetic PostgreSQL connection")
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("DATABASE_URL")
            .expect("S4-DM-ATOMIC requires synthetic PostgreSQL; unset is NOT RUN");
        let schema = format!("dm_atomic_{}", Uuid::now_v7().simple());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let observer = scoped_pool(&url, &schema, "dm_atomic_observer", 2).await;
        MIGRATOR.run(&observer).await.unwrap();
        let pool = scoped_pool(&url, &schema, &schema, 5).await;
        Self {
            observer,
            pool,
            schema,
        }
    }

    async fn user(&self) -> (Uuid, String, String) {
        let id = Uuid::now_v7();
        let handle = id.simple().to_string();
        sqlx::query("INSERT INTO users (id, handle, email, display_name) VALUES ($1, $2, $2 || '@example.invalid', 'Synthetic')")
            .bind(id).bind(&handle).execute(&self.observer).await.unwrap();
        let token = format!("synthetic-{id}");
        sqlx::query("INSERT INTO sessions (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, now() + interval '1 hour')")
            .bind(Uuid::now_v7()).bind(id).bind(Sha256::digest(token.as_bytes()).to_vec())
            .execute(&self.observer).await.unwrap();
        (id, handle, token)
    }

    fn router(&self) -> Router {
        let (hub, _) = tokio::sync::broadcast::channel(HUB_CAPACITY);
        build_router(AppState {
            pool: Some(self.pool.clone()),
            hub,
        })
    }

    async fn wait_for_blocked(&self, count: i64) {
        timeout(DEADLINE, async {
            loop {
                let blocked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pg_stat_activity WHERE application_name = $1 AND wait_event_type = 'Lock'")
                    .bind(&self.schema).fetch_one(&self.observer).await.unwrap();
                if blocked == count { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("all requests must reach a real PostgreSQL lock barrier");
    }

    async fn empty(&self) {
        for table in ["conversations", "conversation_participants"] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&self.observer)
                .await
                .unwrap();
            assert_eq!(count, 0, "failed creation must leave no {table}");
        }
    }

    async fn close(self) {
        self.pool.close().await;
        // Only the randomly named synthetic schema created by this fixture.
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.observer)
            .await
            .unwrap();
        self.observer.close().await;
    }
}

async fn open_dm(app: Router, token: String, peer: String) -> Uuid {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/conversations/dm")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"peer_handle": peer}).to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["kind"], "dm");
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert_eq!(value["members"].as_array().unwrap().len(), 2);
    value["id"].as_str().unwrap().parse().unwrap()
}

async fn race(reversed: bool) {
    let f = Fixture::new().await;
    let (a, ah, at) = f.user().await;
    let (b, bh, bt) = f.user().await;
    let mut blocker = f.observer.begin().await.unwrap();
    // SHARE allows both baseline lookups to miss, but stops INSERT. With the
    // fix, only the leader reaches INSERT; followers wait on its pair lock.
    sqlx::query("LOCK TABLE conversations IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let mut requests = JoinSet::new();
    for index in 0..5 {
        let (token, peer) = if reversed && index % 2 == 1 {
            (bt.clone(), ah.clone())
        } else {
            (at.clone(), bh.clone())
        };
        requests.spawn(open_dm(f.router(), token, peer));
    }
    f.wait_for_blocked(5).await;
    blocker.commit().await.unwrap();
    let ids = timeout(DEADLINE, async {
        let mut ids = Vec::new();
        while let Some(result) = requests.join_next().await {
            ids.push(result.unwrap());
        }
        ids
    })
    .await
    .expect("pool of five must complete without nested acquisition");
    assert!(
        ids.iter().all(|id| *id == ids[0]),
        "same unordered pair must return one DM: {ids:?}"
    );
    let mut members: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM conversation_participants WHERE conversation_id = $1 ORDER BY user_id",
    )
    .bind(ids[0])
    .fetch_all(&f.observer)
    .await
    .unwrap();
    let mut expected = vec![a, b];
    expected.sort_unstable();
    members.sort_unstable();
    assert_eq!(members, expected);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
        .fetch_one(&f.observer)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(open_dm(f.router(), bt, ah).await, ids[0]);
    compatible(&f, ids[0], a, b).await;
    f.close().await;
}

async fn compatible(f: &Fixture, conversation: Uuid, a: Uuid, b: Uuid) {
    let (outsider, _, _) = f.user().await;
    let key = Uuid::now_v7();
    let (message, fresh) =
        messaging::send_message(&f.pool, a, conversation, key, b"synthetic-opaque", None)
            .await
            .unwrap();
    assert!(fresh);
    let (retry, fresh) =
        messaging::send_message(&f.pool, a, conversation, key, b"synthetic-opaque", None)
            .await
            .unwrap();
    assert!(!fresh);
    assert_eq!(message.id, retry.id);
    let history = messaging::message_history(&f.pool, b, conversation, 0, 100)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, message.id);
    assert_eq!(history[0].seq, 1);
    assert!(matches!(
        messaging::message_history(&f.pool, outsider, conversation, 0, 100).await,
        Err(messaging::MessagingError::NotMember)
    ));
    assert!(matches!(
        messaging::send_message(
            &f.pool,
            outsider,
            conversation,
            Uuid::now_v7(),
            b"synthetic",
            None
        )
        .await,
        Err(messaging::MessagingError::NotMember)
    ));
    let events = messaging::events_after(&f.pool, b, None, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].payload["conversation_id"],
        conversation.to_string()
    );
    assert!(messaging::events_after(&f.pool, outsider, None, 100)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn parallel_same_pair_is_one_dm() {
    race(false).await;
}

#[tokio::test]
async fn parallel_reversed_pair_is_one_dm() {
    race(true).await;
}

#[tokio::test]
async fn failed_participant_insert_rolls_back_every_creation_kind() {
    let f = Fixture::new().await;
    let (a, _, _) = f.user().await;
    let (b, _, _) = f.user().await;
    for kind in ["group", "dm", "channel"] {
        let result =
            messaging::create_conversation(&f.pool, a, kind, &[b, Uuid::now_v7(), a]).await;
        assert!(
            matches!(result, Err(messaging::MessagingError::Database(sqlx::Error::Database(ref error))) if error.code().as_deref() == Some("23503"))
        );
        f.empty().await;
    }
    assert!(messaging::find_or_create_dm(&f.pool, a, Uuid::now_v7())
        .await
        .is_err());
    f.empty().await;
    f.close().await;
}

#[tokio::test]
async fn group_and_self_dm_keep_existing_membership_policy() {
    let f = Fixture::new().await;
    let (a, _, _) = f.user().await;
    let (b, _, _) = f.user().await;
    let group = messaging::create_conversation(&f.pool, a, "group", &[b, b])
        .await
        .unwrap();
    // Preserve the existing response vector, including caller-supplied repeats.
    assert_eq!(group.members, vec![b, b, a]);
    compatible(&f, group.id, a, b).await;
    let first = messaging::find_or_create_dm(&f.pool, a, a).await.unwrap();
    let second = messaging::find_or_create_dm(&f.pool, a, a).await.unwrap();
    assert_eq!(first.members, vec![a]);
    assert_eq!(second.members, vec![a]);
    assert_ne!(
        first.id, second.id,
        "existing self-DM policy creates singleton conversations"
    );
    f.close().await;
}

#[tokio::test]
async fn cancelled_creation_does_not_publish_partial_membership() {
    let f = Fixture::new().await;
    let (a, _, _) = f.user().await;
    let (b, _, _) = f.user().await;
    let mut blocker = f.observer.begin().await.unwrap();
    sqlx::query("LOCK TABLE conversation_participants IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let pool = f.pool.clone();
    let request =
        tokio::spawn(async move { messaging::create_conversation(&pool, a, "group", &[b]).await });
    f.wait_for_blocked(1).await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    blocker.commit().await.unwrap();
    // SQLx queues rollback on drop; draining the application pool completes it.
    f.pool.close().await;
    f.empty().await;
    f.close().await;
}

#[tokio::test]
async fn cancelled_dm_leader_releases_pair_for_waiter() {
    let f = Fixture::new().await;
    let (a, _, _) = f.user().await;
    let (b, _, _) = f.user().await;
    let mut blocker = f.observer.begin().await.unwrap();
    sqlx::query("LOCK TABLE conversation_participants IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let pool = f.pool.clone();
    let leader = tokio::spawn(async move { messaging::find_or_create_dm(&pool, a, b).await });
    f.wait_for_blocked(1).await;
    let pool = f.pool.clone();
    let waiter = tokio::spawn(async move { messaging::find_or_create_dm(&pool, b, a).await });
    f.wait_for_blocked(2).await;
    leader.abort();
    assert!(leader.await.unwrap_err().is_cancelled());
    blocker.commit().await.unwrap();
    let dm = timeout(DEADLINE, waiter)
        .await
        .expect("cancelled leader must release transaction lock")
        .unwrap()
        .unwrap();
    let reopened = messaging::find_or_create_dm(&f.pool, a, b).await.unwrap();
    assert_eq!(dm.id, reopened.id);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
        .fetch_one(&f.observer)
        .await
        .unwrap();
    assert_eq!(count, 1, "cancelled leader must leave no orphan");
    f.close().await;
}

#[tokio::test]
async fn lookup_preserves_historical_records_and_exact_membership() {
    let f = Fixture::new().await;
    let (a, _, _) = f.user().await;
    let (b, _, _) = f.user().await;
    let (c, _, _) = f.user().await;
    let group = messaging::create_conversation(&f.pool, a, "group", &[b])
        .await
        .unwrap();
    let larger = messaging::create_conversation(&f.pool, a, "dm", &[b, c])
        .await
        .unwrap();
    let first = messaging::find_or_create_dm(&f.pool, a, b).await.unwrap();
    assert_ne!(first.id, group.id);
    assert_ne!(first.id, larger.id);
    // Raw creation simulates already-existing duplicate rows. This public
    // helper retains its historical behavior; HTTP DMs use find_or_create_dm.
    let second = messaging::create_conversation(&f.pool, b, "dm", &[a])
        .await
        .unwrap();
    for id in [first.id, second.id] {
        messaging::send_message(&f.pool, a, id, Uuid::now_v7(), b"synthetic-history", None)
            .await
            .unwrap();
    }
    let reopened = messaging::find_or_create_dm(&f.pool, b, a).await.unwrap();
    assert!([first.id, second.id].contains(&reopened.id));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
        .fetch_one(&f.observer)
        .await
        .unwrap();
    assert_eq!(
        count, 4,
        "no consolidation or new DM when an exact pair exists"
    );
    for id in [first.id, second.id] {
        let history = messaging::message_history(&f.pool, b, id, 0, 100)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].ciphertext, b"synthetic-history");
    }
    f.close().await;
}

#[tokio::test]
async fn http_group_list_carries_member_profiles_not_channel_rosters() {
    let f = Fixture::new().await;
    let (a, a_handle, token) = f.user().await;
    let (b, b_handle, _) = f.user().await;
    let (c, _, _) = f.user().await;
    let request = Request::builder()
        .method("POST")
        .uri("/v1/conversations")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"member_handles": [b_handle]}).to_string(),
        ))
        .unwrap();
    let response = f.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let group: Uuid = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let channel = messaging::create_conversation(&f.pool, a, "channel", &[c])
        .await
        .unwrap();

    let request = Request::builder()
        .uri("/v1/conversations")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = f.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let row = |id: Uuid| {
        rows.iter()
            .find(|row| row["id"] == id.to_string())
            .expect("listed")
    };
    let mut profiles = row(group)["member_profiles"].as_array().unwrap().clone();
    profiles.sort_by_key(|p| p["handle"].as_str().unwrap().to_owned());
    let mut expected = vec![
        serde_json::json!({"user_id": a, "handle": a_handle, "display_name": "Synthetic"}),
        serde_json::json!({"user_id": b, "handle": b_handle, "display_name": "Synthetic"}),
    ];
    expected.sort_by_key(|p| p["handle"].as_str().unwrap().to_owned());
    assert_eq!(profiles, expected, "exact wire shape the client reads");
    assert_eq!(row(channel.id)["member_profiles"], serde_json::json!([]));
    f.close().await;
}
