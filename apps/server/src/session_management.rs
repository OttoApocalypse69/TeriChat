//! Account-owned session inventory and individual revocation.
//!
//! Session revocation does not revoke devices or MLS membership. A successful
//! response prevents subsequent authentication; gateway enforcement is separate.
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    errors::AppError,
    state::{AppState, Bearer},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/auth/sessions", get(list))
        .route("/v1/auth/sessions/{id}", delete(revoke))
}

#[derive(Default, Deserialize)]
struct PageQuery {
    after: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct SessionView {
    id: Uuid,
    device_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    is_current: bool,
}

#[derive(Serialize)]
struct SessionPage {
    sessions: Vec<SessionView>,
    next_cursor: Option<Uuid>,
}

fn database_error(_: sqlx::Error) -> AppError {
    // Do not log driver errors: database diagnostics may contain row values.
    tracing::error!("session management database failure");
    AppError::Internal
}

async fn list(
    State(state): State<AppState>,
    bearer: Bearer,
    Query(query): Query<PageQuery>,
) -> Result<Json<SessionPage>, AppError> {
    let limit = query.limit.unwrap_or(100);
    if limit <= 0 {
        return Err(AppError::BadRequest("limit must be positive".to_owned()));
    }
    let limit = limit.min(100);
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    // Keep caller authorization valid through the snapshot read. This takes
    // only one row lock, and never upgrades it, so it cannot introduce a
    // reciprocal caller/target lock cycle with revocation.
    sqlx::query("SELECT id FROM sessions WHERE id = $1 AND user_id = $2 FOR SHARE")
        .bind(bearer.session_id())
        .bind(bearer.user_id())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
    require_live(&mut tx, bearer.user_id(), bearer.session_id()).await?;
    let mut sessions: Vec<SessionView> = sqlx::query_as(
        r"SELECT id, device_id, created_at, expires_at, id = $2 AS is_current
          FROM sessions WHERE user_id = $1 AND revoked_at IS NULL
          AND expires_at > statement_timestamp() AND ($3::uuid IS NULL OR id > $3)
          ORDER BY id LIMIT $4",
    )
    .bind(bearer.user_id())
    .bind(bearer.session_id())
    .bind(query.after)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(database_error)?;
    let limit = usize::try_from(limit).map_err(|_| AppError::Internal)?;
    let next_cursor = if sessions.len() > limit {
        sessions.truncate(limit);
        sessions.last().map(|session| session.id)
    } else {
        None
    };
    tx.commit().await.map_err(database_error)?;
    Ok(Json(SessionPage {
        sessions,
        next_cursor,
    }))
}

async fn require_live(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<(), AppError> {
    // A fresh statement after locks is essential: now() is transaction-start
    // time and would accept sessions that expired while waiting for a lock.
    let live: bool = sqlx::query_scalar(
        r"SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1 AND user_id = $2
           AND revoked_at IS NULL AND expires_at > statement_timestamp())",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_error)?;
    if live {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

async fn revoke(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(target): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    revoke_owned(pool, bearer.user_id(), bearer.session_id(), target).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_owned(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    caller: Uuid,
    target: Uuid,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(database_error)?;
    // Lock caller and owned target in immutable UUID order, even when the
    // target is already revoked/expired. Reciprocal revocations serialize;
    // the loser rechecks its now-revoked caller and cannot revoke the winner.
    let owned: Vec<Uuid> = sqlx::query_scalar(
        r"SELECT id FROM sessions WHERE user_id = $1 AND (id = $2 OR id = $3)
           ORDER BY id FOR UPDATE",
    )
    .bind(user_id)
    .bind(caller)
    .bind(target)
    .fetch_all(&mut *tx)
    .await
    .map_err(database_error)?;
    require_live(&mut tx, user_id, caller).await?;
    if !owned.contains(&target) {
        return Err(AppError::NotFound("session not found".to_owned()));
    }
    sqlx::query("UPDATE sessions SET revoked_at = statement_timestamp() WHERE id = $1 AND revoked_at IS NULL")
        .bind(target)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use sha2::{Digest as _, Sha256};
    use tower::ServiceExt as _;

    pub(crate) async fn fixture() -> Option<(sqlx::PgPool, AppState, Uuid)> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("SKIPPED: session database test (DATABASE_URL unset)");
            return None;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("test database");
        crate::MIGRATOR.run(&pool).await.unwrap();
        let user = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users (id, handle, email, display_name) VALUES ($1, $2, $3, 'Synthetic')",
        )
        .bind(user)
        .bind(user.simple().to_string())
        .bind(format!("{user}@example.invalid"))
        .execute(&pool)
        .await
        .unwrap();
        let (hub, _) = tokio::sync::broadcast::channel(crate::HUB_CAPACITY);
        let state = AppState {
            pool: Some(pool.clone()),
            hub,
        };
        Some((pool, state, user))
    }

    pub(crate) async fn session(pool: &sqlx::PgPool, user: Uuid) -> (Uuid, String) {
        let id = Uuid::now_v7();
        let token = format!("synthetic-session-{id}");
        sqlx::query("INSERT INTO sessions (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, now() + interval '1 hour')")
            .bind(id).bind(user).bind(Sha256::digest(token.as_bytes()).as_slice())
            .execute(pool).await.unwrap();
        (id, token)
    }

    async fn request(
        state: &AppState,
        method: &str,
        uri: &str,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        let response = crate::build_router(state.clone())
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn session_inventory_caps_and_validates_pages() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let (caller, token) = session(&pool, user).await;
        assert_eq!(
            request(&state, "GET", "/v1/auth/sessions", None).await.0,
            StatusCode::UNAUTHORIZED
        );
        for query in ["limit=0", "limit=-1", "after=broken"] {
            assert_eq!(
                request(
                    &state,
                    "GET",
                    &format!("/v1/auth/sessions?{query}"),
                    Some(&token)
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        let mut expected = vec![caller.to_string()];
        for _ in 0..101 {
            expected.push(session(&pool, user).await.0.to_string());
        }
        let (_, first) = request(&state, "GET", "/v1/auth/sessions?limit=999", Some(&token)).await;
        let rows = first["sessions"].as_array().unwrap();
        assert_eq!(rows.len(), 100);
        let mut actual = rows
            .iter()
            .map(|row| row["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let (_, second) = request(
            &state,
            "GET",
            &format!(
                "/v1/auth/sessions?after={}",
                first["next_cursor"].as_str().unwrap()
            ),
            Some(&token),
        )
        .await;
        actual.extend(
            second["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_owned()),
        );
        assert_eq!(actual, expected);
        assert!(second["next_cursor"].is_null());
        pool.close().await;
    }
    #[tokio::test]
    async fn session_routes_ownership_pagination_and_revocation() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let (caller, token) = session(&pool, user).await;
        let (target, _) = session(&pool, user).await;
        let (expired, _) = session(&pool, user).await;
        let (revoked, _) = session(&pool, user).await;
        sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second' WHERE id = $1")
            .bind(expired)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
            .bind(revoked)
            .execute(&pool)
            .await
            .unwrap();
        let Some((other_pool, _, other)) = fixture().await else {
            panic!("database disappeared");
        };
        let (foreign, _) = session(&pool, other).await;
        let (status, first) =
            request(&state, "GET", "/v1/auth/sessions?limit=1", Some(&token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(first["sessions"][0]["id"], caller.to_string());
        assert_eq!(first["sessions"][0]["is_current"], true);
        let keys = first["sessions"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            ["created_at", "device_id", "expires_at", "id", "is_current"]
        );
        let (_, second) = request(
            &state,
            "GET",
            &format!(
                "/v1/auth/sessions?limit=1&after={}",
                first["next_cursor"].as_str().unwrap()
            ),
            Some(&token),
        )
        .await;
        assert_eq!(second["sessions"][0]["id"], target.to_string());
        assert_eq!(second["sessions"][0]["is_current"], false);
        assert!(second["next_cursor"].is_null());
        for id in [foreign, Uuid::now_v7()] {
            assert_eq!(
                request(
                    &state,
                    "DELETE",
                    &format!("/v1/auth/sessions/{id}"),
                    Some(&token)
                )
                .await
                .0,
                StatusCode::NOT_FOUND
            );
        }
        for _ in 0..2 {
            assert_eq!(
                request(
                    &state,
                    "DELETE",
                    &format!("/v1/auth/sessions/{target}"),
                    Some(&token)
                )
                .await
                .0,
                StatusCode::NO_CONTENT
            );
        }
        let (_, page) = request(&state, "GET", "/v1/auth/sessions?limit=999", Some(&token)).await;
        assert_eq!(page["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(
            request(
                &state,
                "DELETE",
                &format!("/v1/auth/sessions/{caller}"),
                Some(&token)
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            request(&state, "GET", "/v1/auth/sessions", Some(&token))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        other_pool.close().await;
        pool.close().await;
    }

    #[tokio::test]
    async fn reciprocal_session_revocation_has_one_winner() {
        let Some((pool, _, user)) = fixture().await else {
            return;
        };
        let (a, _) = session(&pool, user).await;
        let (b, _) = session(&pool, user).await;
        let (left, right) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                revoke_owned(&pool, user, a, b),
                revoke_owned(&pool, user, b, a)
            )
        })
        .await
        .expect("no reciprocal lock deadlock");
        assert!(matches!(
            (&left, &right),
            (Ok(()), Err(AppError::Unauthorized)) | (Err(AppError::Unauthorized), Ok(()))
        ));
        let live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sessions WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(live, 1);
        pool.close().await;
    }

    #[tokio::test]
    async fn caller_expiring_while_waiting_cannot_revoke() {
        let Some((pool, _, user)) = fixture().await else {
            return;
        };
        let (caller, _) = session(&pool, user).await;
        let (target, _) = session(&pool, user).await;
        sqlx::query("UPDATE sessions SET expires_at = now() + interval '1 second' WHERE id = $1")
            .bind(caller)
            .execute(&pool)
            .await
            .unwrap();
        let mut blocker = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM sessions WHERE id = $1 FOR UPDATE")
            .bind(caller)
            .execute(&mut *blocker)
            .await
            .unwrap();
        let copy = pool.clone();
        let pending = tokio::spawn(async move { revoke_owned(&copy, user, caller, target).await });
        // Confirm the mutation reached its lock wait before advancing expiry.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE 'SELECT id FROM sessions WHERE user_id%')").fetch_one(&pool).await.unwrap();
                if waiting { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("revocation waiting on caller lock");
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        blocker.commit().await.unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            Err(AppError::Unauthorized)
        ));
        let revoked: bool =
            sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM sessions WHERE id = $1")
                .bind(target)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!revoked);
        pool.close().await;
    }

    #[tokio::test]
    async fn caller_expiring_while_inventory_waits_is_rejected() {
        let Some((pool, state, user)) = fixture().await else {
            return;
        };
        let (caller, token) = session(&pool, user).await;
        sqlx::query("UPDATE sessions SET expires_at = now() + interval '1 second' WHERE id = $1")
            .bind(caller)
            .execute(&pool)
            .await
            .unwrap();
        let mut blocker = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM sessions WHERE id = $1 FOR UPDATE")
            .bind(caller)
            .execute(&mut *blocker)
            .await
            .unwrap();
        let pending =
            tokio::spawn(
                async move { request(&state, "GET", "/v1/auth/sessions", Some(&token)).await },
            );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query = 'SELECT id FROM sessions WHERE id = $1 AND user_id = $2 FOR SHARE')").fetch_one(&pool).await.unwrap();
                if waiting { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("inventory waiting on caller lock");
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        blocker.commit().await.unwrap();
        assert_eq!(pending.await.unwrap().0, StatusCode::UNAUTHORIZED);
        pool.close().await;
    }
}
