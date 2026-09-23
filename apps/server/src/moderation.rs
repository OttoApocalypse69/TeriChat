//! Current ban directory. Reasons are visible only with the existing
//! `BanMembers` permission; this is not a public member roster or audit feed.
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    errors::AppError,
    state::{AppState, Bearer},
    workspaces::{self, WorkspacesError},
};

#[derive(Deserialize)]
struct PageQuery {
    after: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct BanBody {
    user_id: Uuid,
    handle: String,
    display_name: String,
    banned_by: Uuid,
    reason: String,
    banned_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct BanPage {
    bans: Vec<BanBody>,
    next_cursor: Option<Uuid>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/v1/workspaces/{id}/bans", get(list_bans))
}

async fn list_bans(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Result<Json<BanPage>, AppError> {
    let limit = query.limit.unwrap_or(100);
    if limit <= 0 {
        return Err(AppError::BadRequest("limit must be positive".to_owned()));
    }
    let limit = limit.min(100);
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let db = WorkspacesError::Database;
    let mut tx = pool.begin().await.map_err(db)?;
    // Locks are acquired in a fixed order: requesting session, then workspace
    // membership. Neither the session nor its authority can be revoked while
    // the authorized directory snapshot is read. Expiry is rechecked after waits.
    let session: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM sessions WHERE id = $1 AND user_id = $2 FOR SHARE")
            .bind(bearer.session_id())
            .bind(bearer.user_id())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
    if session.is_none() {
        return Err(AppError::Unauthorized);
    }
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(workspace_id)
    .bind(bearer.user_id())
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let live: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sessions WHERE id = $1 AND user_id = $2
         AND revoked_at IS NULL AND expires_at > clock_timestamp())",
    )
    .bind(bearer.session_id())
    .bind(bearer.user_id())
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if !live {
        return Err(AppError::Unauthorized);
    }
    let banned: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspace_bans WHERE workspace_id = $1 AND user_id = $2)",
    )
    .bind(workspace_id)
    .bind(bearer.user_id())
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if banned {
        return Err(WorkspacesError::Banned.into());
    }
    let role = role.ok_or(WorkspacesError::NotMember)?;
    let role = workspaces::parse_role(&role)?;
    if !workspaces::role_has(role, workspaces::Permission::BanMembers) {
        return Err(WorkspacesError::Forbidden.into());
    }
    let mut bans: Vec<BanBody> = sqlx::query_as(
        "SELECT b.user_id, u.handle, u.display_name, b.banned_by, b.reason, b.banned_at
         FROM workspace_bans b JOIN users u ON u.id = b.user_id
         WHERE b.workspace_id = $1 AND ($2::uuid IS NULL OR b.user_id > $2)
         ORDER BY b.user_id LIMIT $3",
    )
    .bind(workspace_id)
    .bind(query.after)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;
    let next_cursor = if i64::try_from(bans.len()).unwrap_or(i64::MAX) > limit {
        bans.pop();
        bans.last().map(|ban| ban.user_id)
    } else {
        None
    };
    tx.commit().await.map_err(db)?;
    Ok(Json(BanPage { bans, next_cursor }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    struct Fixture {
        pool: sqlx::PgPool,
        app: Router,
        workspace: Uuid,
        owner: Uuid,
        tokens: Vec<String>,
        users: Vec<Uuid>,
    }

    async fn fixture() -> Option<Fixture> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("SKIPPED: moderation directory tests (DATABASE_URL unset)");
            return None;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("synthetic DB");
        crate::MIGRATOR.run(&pool).await.expect("migrations");
        let mut tokens = Vec::new();
        let mut users = Vec::new();
        for name in ["owner", "admin", "moderator", "member", "guest", "outside"] {
            let handle = format!("ban{}", Uuid::now_v7().simple());
            // Valid handles are <=32 bytes; this distinct fixture suffix is 28.
            let handle = &handle[..31];
            let user = crate::auth::create_user(
                &pool,
                handle,
                &format!("{handle}@example.invalid"),
                name,
                "synthetic-ban-password",
            )
            .await
            .expect("account");
            tokens.push(
                crate::auth::login(&pool, handle, "synthetic-ban-password")
                    .await
                    .expect("session")
                    .token,
            );
            users.push(user.id);
        }
        let owner = users[0];
        let workspace = workspaces::create_workspace(&pool, owner, "Ban directory")
            .await
            .expect("workspace")
            .id;
        for (idx, role) in [
            (1, workspaces::Role::Admin),
            (2, workspaces::Role::Moderator),
            (3, workspaces::Role::Member),
            (4, workspaces::Role::Guest),
        ] {
            workspaces::add_member(&pool, owner, workspace, users[idx], role)
                .await
                .expect("membership");
        }
        let (hub, _) = tokio::sync::broadcast::channel(crate::HUB_CAPACITY);
        let app = crate::build_router(AppState::new(
            Some(pool.clone()),
            hub,
            std::env::temp_dir().join("terichat-test-attachments"),
        ));
        Some(Fixture {
            pool,
            app,
            workspace,
            owner,
            tokens,
            users,
        })
    }

    async fn request(
        app: Router,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .oneshot(
                builder
                    .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn seed_bans(f: &Fixture, count: usize, workspace: Uuid) -> Vec<Uuid> {
        let mut ids = Vec::new();
        for _ in 0..count {
            let id = Uuid::now_v7();
            let handle = format!("b{}", &id.simple().to_string()[..30]);
            sqlx::query("INSERT INTO users(id,handle,email,display_name) VALUES($1,$2,$3,'Synthetic banned user')")
                .bind(id).bind(&handle).bind(format!("{handle}@example.invalid"))
                .execute(&f.pool).await.expect("synthetic target");
            sqlx::query("INSERT INTO workspace_bans(workspace_id,user_id,banned_by,reason) VALUES($1,$2,$3,'synthetic reason')")
                .bind(workspace).bind(id).bind(f.owner).execute(&f.pool).await.expect("ban fixture");
            ids.push(id);
        }
        ids.sort_unstable();
        ids
    }

    #[tokio::test]
    async fn bans_directory_permissions_pages_and_private_fields() {
        let Some(f) = fixture().await else {
            return;
        };
        let other = workspaces::create_workspace(&f.pool, f.owner, "Other scope")
            .await
            .expect("other")
            .id;
        let expected = seed_bans(&f, 103, f.workspace).await;
        let foreign = seed_bans(&f, 1, other).await[0];
        let uri = format!("/v1/workspaces/{}/bans", f.workspace);
        assert_eq!(
            request(f.app.clone(), "GET", &uri, None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
        for idx in 2..6 {
            let denied = request(f.app.clone(), "GET", &uri, Some(&f.tokens[idx]), None).await;
            assert_eq!(denied.0, StatusCode::FORBIDDEN);
            assert!(denied.1.get("bans").is_none());
        }
        for idx in 0..2 {
            let (status, first) = request(
                f.app.clone(),
                "GET",
                &format!("{uri}?limit=999"),
                Some(&f.tokens[idx]),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let rows = first["bans"].as_array().expect("rows");
            assert_eq!(rows.len(), 100);
            let mut keys: Vec<_> = rows[0]
                .as_object()
                .expect("object")
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                [
                    "banned_at",
                    "banned_by",
                    "display_name",
                    "handle",
                    "reason",
                    "user_id"
                ]
            );
            let cursor = first["next_cursor"].as_str().expect("cursor");
            let (status, second) = request(
                f.app.clone(),
                "GET",
                &format!("{uri}?after={cursor}"),
                Some(&f.tokens[idx]),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(second["next_cursor"].is_null());
            let ids: Vec<Uuid> = rows
                .iter()
                .chain(second["bans"].as_array().expect("second page"))
                .map(|v| v["user_id"].as_str().expect("id").parse().expect("uuid"))
                .collect();
            assert_eq!(ids, expected);
        }
        let scoped = request(
            f.app.clone(),
            "GET",
            &format!("{uri}?after={foreign}"),
            Some(&f.tokens[0]),
            None,
        )
        .await;
        assert_eq!(scoped.0, StatusCode::OK);
        assert!(scoped.1["bans"].as_array().expect("page").is_empty());
        for query in ["limit=0", "limit=-1", "after=bad", "limit=bad"] {
            assert_eq!(
                request(
                    f.app.clone(),
                    "GET",
                    &format!("{uri}?{query}"),
                    Some(&f.tokens[0]),
                    None
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        let missing = format!("/v1/workspaces/{}/bans", Uuid::now_v7());
        assert_eq!(
            request(f.app.clone(), "GET", &missing, Some(&f.tokens[0]), None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn bans_directory_preserves_post_and_unban_routes() {
        let Some(f) = fixture().await else {
            return;
        };
        let uri = format!("/v1/workspaces/{}/bans", f.workspace);
        let banned = request(
            f.app.clone(),
            "POST",
            &uri,
            Some(&f.tokens[0]),
            Some(json!({"user_id":f.users[3],"reason":"synthetic moderation"})),
        )
        .await;
        assert!(banned.0.is_success());
        let listed = request(f.app.clone(), "GET", &uri, Some(&f.tokens[1]), None).await;
        assert_eq!(listed.1["bans"][0]["user_id"], f.users[3].to_string());
        let unban = format!("{uri}/{}", f.users[3]);
        assert!(
            request(f.app.clone(), "DELETE", &unban, Some(&f.tokens[0]), None)
                .await
                .0
                .is_success()
        );
        assert_eq!(
            request(f.app.clone(), "GET", &uri, Some(&f.tokens[0]), None)
                .await
                .1["bans"],
            json!([])
        );
        assert!(workspaces::role_of(&f.pool, f.workspace, f.users[3])
            .await
            .expect("membership")
            .is_none());
        // Inconsistent stale membership must not let a banned manager read reasons.
        sqlx::query("INSERT INTO workspace_bans(workspace_id,user_id,banned_by) VALUES($1,$2,$3)")
            .bind(f.workspace)
            .bind(f.users[1])
            .bind(f.owner)
            .execute(&f.pool)
            .await
            .expect("stale ban");
        assert_eq!(
            request(f.app.clone(), "GET", &uri, Some(&f.tokens[1]), None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn bans_directory_rechecks_role_after_lock_wait() {
        let Some(f) = fixture().await else {
            return;
        };
        seed_bans(&f, 1, f.workspace).await;
        let mut demotion = f.pool.begin().await.expect("demotion tx");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *demotion)
            .await
            .expect("pid");
        sqlx::query(
            "UPDATE workspace_members SET role='member' WHERE workspace_id=$1 AND user_id=$2",
        )
        .bind(f.workspace)
        .bind(f.users[1])
        .execute(&mut *demotion)
        .await
        .expect("uncommitted demotion");
        let app = f.app.clone();
        let token = f.tokens[1].clone();
        let uri = format!("/v1/workspaces/{}/bans", f.workspace);
        let pending =
            tokio::spawn(async move { request(app, "GET", &uri, Some(&token), None).await });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))")
                    .bind(pid).fetch_one(&f.pool).await.expect("lock observation");
                if blocked { break; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.expect("request waits on authority row");
        demotion.commit().await.expect("demotion commits");
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
            .await
            .expect("response bounded")
            .expect("request task");
        assert_eq!(result.0, StatusCode::FORBIDDEN);
        assert!(result.1.get("bans").is_none());
    }
}
