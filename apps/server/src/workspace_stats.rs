//! Caller-private activity over a workspace's current channels.
//! Existing metadata counters are eventually consistent with outbox delivery.
//! Zero-count channels are included; deleted channels cease to contribute.
use crate::{
    errors::AppError,
    state::{AppState, Bearer},
    workspaces::WorkspacesError,
};
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize)]
struct WorkspaceStats {
    user_id: Uuid,
    workspace_id: Uuid,
    message_count: i64,
    last_message_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct ChannelStats {
    channel_id: Uuid,
    conversation_id: Uuid,
    name: String,
    message_count: i64,
    last_message_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct ChannelPage {
    channels: Vec<ChannelStats>,
    next_cursor: Option<Uuid>,
}

#[derive(Deserialize)]
struct PageParams {
    after: Option<Uuid>,
    limit: Option<i64>,
}

// Match workspace roster locking: revocation either commits before this read
// (and is rejected), or waits until its authorized query has finished.
async fn authorized_read(
    pool: &sqlx::PgPool,
    user: Uuid,
    session: Uuid,
    workspace: Uuid,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, AppError> {
    let mut tx = pool.begin().await.map_err(WorkspacesError::Database)?;
    let live: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL FOR SHARE",
    )
    .bind(session)
    .bind(user)
    .fetch_optional(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    live.ok_or(AppError::Unauthorized)?;
    let member: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM workspace_members WHERE workspace_id=$1 AND user_id=$2 FOR SHARE",
    )
    .bind(workspace)
    .bind(user)
    .fetch_optional(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    let banned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM workspace_bans WHERE workspace_id=$1 AND user_id=$2)",
    )
    .bind(workspace)
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    let unexpired: bool =
        sqlx::query_scalar("SELECT expires_at>clock_timestamp() FROM sessions WHERE id=$1")
            .bind(session)
            .fetch_one(&mut *tx)
            .await
            .map_err(WorkspacesError::Database)?;
    if !unexpired {
        return Err(AppError::Unauthorized);
    }
    if banned {
        return Err(WorkspacesError::Banned.into());
    }
    member.ok_or(WorkspacesError::NotMember)?;
    Ok(tx)
}

async fn own_workspace_stats(
    pool: &sqlx::PgPool,
    user: Uuid,
    session: Uuid,
    workspace: Uuid,
) -> Result<WorkspaceStats, AppError> {
    let mut tx = authorized_read(pool, user, session, workspace).await?;
    let (message_count, last_message_at) = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
        "SELECT COALESCE(SUM(s.message_count),0)::bigint, MAX(s.last_message_at)
         FROM channels c LEFT JOIN user_conversation_stats s
         ON s.conversation_id=c.conversation_id AND s.user_id=$2
         WHERE c.workspace_id=$1",
    )
    .bind(workspace)
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .map_err(WorkspacesError::Database)?;
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(WorkspaceStats {
        user_id: user,
        workspace_id: workspace,
        message_count,
        last_message_at,
    })
}

async fn own_channel_stats(
    pool: &sqlx::PgPool,
    user: Uuid,
    session: Uuid,
    workspace: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<ChannelPage, AppError> {
    let mut tx = authorized_read(pool, user, session, workspace).await?;
    if limit <= 0 {
        return Err(WorkspacesError::BadInput("limit must be positive".to_owned()).into());
    }
    let limit = limit.min(100);
    let mut channels: Vec<ChannelStats> = sqlx::query_as(
        "SELECT c.id AS channel_id,c.conversation_id,c.name,COALESCE(s.message_count,0) AS message_count,s.last_message_at
         FROM channels c LEFT JOIN user_conversation_stats s
         ON s.conversation_id=c.conversation_id AND s.user_id=$2
         WHERE c.workspace_id=$1 AND ($3::uuid IS NULL OR c.id>$3)
         ORDER BY c.id LIMIT $4")
        .bind(workspace).bind(user).bind(after).bind(limit+1)
        .fetch_all(&mut *tx).await.map_err(WorkspacesError::Database)?;
    let next_cursor = if i64::try_from(channels.len()).unwrap_or(i64::MAX) > limit {
        channels.pop();
        channels.last().map(|channel| channel.channel_id)
    } else {
        None
    };
    tx.commit().await.map_err(WorkspacesError::Database)?;
    Ok(ChannelPage {
        channels,
        next_cursor,
    })
}

async fn aggregate(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkspaceStats>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    Ok(Json(
        own_workspace_stats(pool, bearer.user_id(), bearer.session_id(), id).await?,
    ))
}

async fn channels(
    State(state): State<AppState>,
    bearer: Bearer,
    Path(id): Path<Uuid>,
    Query(page): Query<PageParams>,
) -> Result<Json<ChannelPage>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    Ok(Json(
        own_channel_stats(
            pool,
            bearer.user_id(),
            bearer.session_id(),
            id,
            page.after,
            page.limit.unwrap_or(100),
        )
        .await?,
    ))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/stats/me", get(aggregate))
        .route("/v1/workspaces/{id}/stats/me/channels", get(channels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth, messaging, stats,
        workspaces::{self, Role},
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn session(pool: &sqlx::PgPool, user: Uuid) -> Uuid {
        sqlx::query_scalar("INSERT INTO sessions (id,user_id,token_hash,expires_at) VALUES ($1,$2,$3,clock_timestamp()+interval '1 hour') RETURNING id")
            .bind(Uuid::now_v7()).bind(user).bind(Uuid::now_v7().simple().to_string().into_bytes())
            .fetch_one(pool).await.unwrap()
    }

    async fn own_workspace_stats(
        pool: &sqlx::PgPool,
        user: Uuid,
        workspace: Uuid,
    ) -> Result<WorkspaceStats, AppError> {
        super::own_workspace_stats(pool, user, session(pool, user).await, workspace).await
    }

    async fn own_channel_stats(
        pool: &sqlx::PgPool,
        user: Uuid,
        workspace: Uuid,
        after: Option<Uuid>,
        limit: i64,
    ) -> Result<ChannelPage, AppError> {
        super::own_channel_stats(
            pool,
            user,
            session(pool, user).await,
            workspace,
            after,
            limit,
        )
        .await
    }

    async fn pool() -> sqlx::PgPool {
        let url =
            std::env::var("DATABASE_URL").expect("workspace stats tests require DATABASE_URL");
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("workspace_stats_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .unwrap();
        crate::MIGRATOR.run(&pool).await.unwrap();
        pool
    }

    async fn user(pool: &sqlx::PgPool, name: &str) -> Uuid {
        auth::create_user(
            pool,
            name,
            &format!("{name}@example.invalid"),
            name,
            "synthetic-stats-password",
        )
        .await
        .unwrap()
        .id
    }

    async fn count(pool: &sqlx::PgPool, user: Uuid, conversation: Uuid) {
        let (message, _) = messaging::send_message(
            pool,
            user,
            conversation,
            Uuid::now_v7(),
            b"synthetic-envelope",
            None,
        )
        .await
        .unwrap();
        let (id, topic, payload): (Uuid, String, serde_json::Value) = sqlx::query_as(
            "SELECT id,topic,payload FROM outbox WHERE payload->'data'->>'message_id'=$1",
        )
        .bind(message.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(stats::process_event(pool, id, &topic, &payload)
            .await
            .unwrap());
        assert!(!stats::process_event(pool, id, &topic, &payload)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn current_channels_scope_counters_and_rejoin() {
        let pool = pool().await;
        let owner = user(&pool, "owner").await;
        let peer = user(&pool, "peer").await;
        let one = workspaces::create_workspace(&pool, owner, "one")
            .await
            .unwrap();
        let two = workspaces::create_workspace(&pool, owner, "two")
            .await
            .unwrap();
        workspaces::add_member(&pool, owner, one.id, peer, Role::Member)
            .await
            .unwrap();
        let a = workspaces::create_channel(&pool, owner, one.id, "a")
            .await
            .unwrap();
        let zero = workspaces::create_channel(&pool, owner, one.id, "zero")
            .await
            .unwrap();
        let foreign = workspaces::create_channel(&pool, owner, two.id, "foreign")
            .await
            .unwrap();
        let dm = messaging::find_or_create_dm(&pool, owner, peer)
            .await
            .unwrap();
        for (user, conversation) in [
            (owner, a.conversation_id),
            (peer, a.conversation_id),
            (owner, foreign.conversation_id),
            (owner, dm.id),
        ] {
            count(&pool, user, conversation).await;
        }
        let aggregate = own_workspace_stats(&pool, owner, one.id).await.unwrap();
        assert_eq!(aggregate.message_count, 1);
        assert_eq!(aggregate.user_id, owner);
        assert_eq!(aggregate.workspace_id, one.id);
        let page = own_channel_stats(&pool, owner, one.id, None, 100)
            .await
            .unwrap();
        assert_eq!(page.channels.len(), 2);
        assert_eq!(page.channels[0].channel_id, a.id);
        assert_eq!(page.channels[0].last_message_at, aggregate.last_message_at);
        assert_eq!(page.channels[1].channel_id, zero.id);
        assert_eq!(page.channels[1].message_count, 0);
        assert_eq!(page.channels[1].last_message_at, None);
        workspaces::set_role(&pool, owner, one.id, peer, Role::Guest)
            .await
            .unwrap();
        assert_eq!(
            own_workspace_stats(&pool, peer, one.id)
                .await
                .unwrap()
                .message_count,
            1
        );
        assert!(matches!(
            own_workspace_stats(&pool, peer, two.id).await,
            Err(AppError::Denied(_))
        ));
        workspaces::ban_member(&pool, owner, one.id, peer, "synthetic ban")
            .await
            .unwrap();
        assert!(matches!(
            own_channel_stats(&pool, peer, one.id, None, 100).await,
            Err(AppError::Denied(_))
        ));
        workspaces::unban(&pool, owner, one.id, peer).await.unwrap();
        assert!(matches!(
            own_workspace_stats(&pool, peer, one.id).await,
            Err(AppError::Denied(_))
        ));
        workspaces::add_member(&pool, owner, one.id, peer, Role::Guest)
            .await
            .unwrap();
        assert_eq!(
            own_workspace_stats(&pool, peer, one.id)
                .await
                .unwrap()
                .message_count,
            1
        );
        workspaces::delete_channel(&pool, owner, a.id)
            .await
            .unwrap();
        let empty = own_workspace_stats(&pool, owner, one.id).await.unwrap();
        assert_eq!(empty.message_count, 0);
        assert_eq!(empty.last_message_at, None);
        assert_eq!(
            own_channel_stats(&pool, owner, one.id, None, 100)
                .await
                .unwrap()
                .channels
                .len(),
            1
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn pagination_is_bounded_and_exclusive() {
        let pool = pool().await;
        let owner = user(&pool, "owner").await;
        let workspace = workspaces::create_workspace(&pool, owner, "page")
            .await
            .unwrap()
            .id;
        let mut ids = Vec::new();
        for i in 0..103 {
            ids.push(
                workspaces::create_channel(&pool, owner, workspace, &format!("c{i}"))
                    .await
                    .unwrap()
                    .id,
            );
        }
        for limit in [0, -1] {
            assert!(matches!(
                own_channel_stats(&pool, owner, workspace, None, limit).await,
                Err(AppError::BadRequest(_))
            ));
        }
        let first = own_channel_stats(&pool, owner, workspace, None, i64::MAX)
            .await
            .unwrap();
        assert_eq!(first.channels.len(), 100);
        assert_eq!(first.next_cursor, Some(ids[99]));
        let second = own_channel_stats(&pool, owner, workspace, first.next_cursor, 100)
            .await
            .unwrap();
        assert_eq!(
            second
                .channels
                .iter()
                .map(|c| c.channel_id)
                .collect::<Vec<_>>(),
            ids[100..]
        );
        assert_eq!(second.next_cursor, None);
        assert!(
            own_channel_stats(&pool, owner, workspace, Some(ids[102]), 100)
                .await
                .unwrap()
                .channels
                .is_empty()
        );
        assert_eq!(
            own_channel_stats(&pool, owner, workspace, None, 1)
                .await
                .unwrap()
                .next_cursor,
            Some(ids[0])
        );
        pool.close().await;
    }

    async fn request(
        app: Router,
        uri: &str,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder().uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
            }),
        )
    }

    #[tokio::test]
    async fn routes_auth_scope_and_private_shape() {
        let pool = pool().await;
        let owner = user(&pool, "owner").await;
        let guest = user(&pool, "guest").await;
        let workspace = workspaces::create_workspace(&pool, owner, "private")
            .await
            .unwrap()
            .id;
        let token = auth::login(&pool, "guest", "synthetic-stats-password")
            .await
            .unwrap()
            .token;
        let (hub, _) = tokio::sync::broadcast::channel(crate::HUB_CAPACITY);
        let app = crate::routes::build_router(AppState {
            pool: Some(pool.clone()),
            hub,
        });
        for suffix in ["", "/channels"] {
            let uri = format!("/v1/workspaces/{workspace}/stats/me{suffix}");
            assert_eq!(
                request(app.clone(), &uri, None).await.0,
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                request(app.clone(), &uri, Some("invalid-synthetic-token"))
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
            let denied = request(app.clone(), &uri, Some(&token)).await;
            let unknown = request(
                app.clone(),
                &format!("/v1/workspaces/{}/stats/me{suffix}", Uuid::now_v7()),
                Some(&token),
            )
            .await;
            assert_eq!(denied.0, StatusCode::FORBIDDEN);
            assert_eq!(denied, unknown);
        }
        workspaces::add_member(&pool, owner, workspace, guest, Role::Guest)
            .await
            .unwrap();
        let uri = format!("/v1/workspaces/{workspace}/stats/me");
        let allowed = request(app.clone(), &uri, Some(&token)).await;
        assert_eq!(allowed.0, StatusCode::OK);
        assert_eq!(
            allowed.1,
            serde_json::json!({"user_id":guest,"workspace_id":workspace,"message_count":0,"last_message_at":null})
        );
        assert_eq!(
            request(app.clone(), &format!("{uri}/channels"), Some(&token))
                .await
                .1,
            serde_json::json!({"channels":[],"next_cursor":null})
        );
        for query in ["limit=0", "limit=-1", "limit=wat", "after=not-a-uuid"] {
            assert_eq!(
                request(
                    app.clone(),
                    &format!("{uri}/channels?{query}"),
                    Some(&token)
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        workspaces::ban_member(
            &pool,
            owner,
            workspace,
            guest,
            "synthetic reason never returned",
        )
        .await
        .unwrap();
        let banned = request(app.clone(), &uri, Some(&token)).await;
        assert_eq!(banned.0, StatusCode::FORBIDDEN);
        assert!(!banned.1.to_string().contains("synthetic reason"));
        pool.close().await;
    }

    #[tokio::test]
    async fn session_revocation_and_expiry_after_wait_deny_both_reads() {
        let pool = pool().await;
        let owner = user(&pool, "owner").await;
        let workspace = workspaces::create_workspace(&pool, owner, "session-race")
            .await
            .unwrap()
            .id;
        for (channel_page, expire) in [(false, false), (true, false), (false, true), (true, true)] {
            let session = session(&pool, owner).await;
            let mut blocker = pool.begin().await.unwrap();
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
            if expire {
                sqlx::query("UPDATE sessions SET expires_at=clock_timestamp()+interval '1 second' WHERE id=$1")
                    .bind(session).execute(&pool).await.unwrap();
                sqlx::query("SELECT user_id FROM workspace_members WHERE workspace_id=$1 AND user_id=$2 FOR UPDATE")
                    .bind(workspace).bind(owner).fetch_one(&mut *blocker).await.unwrap();
            } else {
                sqlx::query("UPDATE sessions SET revoked_at=clock_timestamp() WHERE id=$1")
                    .bind(session)
                    .execute(&mut *blocker)
                    .await
                    .unwrap();
            }
            let read_pool = pool.clone();
            let read = tokio::spawn(async move {
                if channel_page {
                    super::own_channel_stats(&read_pool, owner, session, workspace, None, 100)
                        .await
                        .map(|_| ())
                } else {
                    super::own_workspace_stats(&read_pool, owner, session, workspace)
                        .await
                        .map(|_| ())
                }
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))")
                        .bind(pid).fetch_one(&pool).await.unwrap();
                    assert!(!read.is_finished(), "reader bypassed authorization lock");
                    if waiting { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.expect("reader waits on real lock");
            if expire {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let expired: bool = sqlx::query_scalar(
                            "SELECT expires_at<=clock_timestamp() FROM sessions WHERE id=$1",
                        )
                        .bind(session)
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                        if expired {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
            }
            blocker.commit().await.unwrap();
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(5), read)
                    .await
                    .unwrap()
                    .unwrap(),
                Err(AppError::Unauthorized)
            ));
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn pending_revocation_is_rechecked_after_lock_wait() {
        let pool = pool().await;
        let owner = user(&pool, "owner").await;
        let member = user(&pool, "member").await;
        let workspace = workspaces::create_workspace(&pool, owner, "race")
            .await
            .unwrap()
            .id;
        for (channel_page, ban) in [(false, false), (true, false), (false, true), (true, true)] {
            workspaces::unban(&pool, owner, workspace, member)
                .await
                .unwrap();
            workspaces::add_member(&pool, owner, workspace, member, Role::Member)
                .await
                .unwrap();
            let mut revoke = pool.begin().await.unwrap();
            let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *revoke)
                .await
                .unwrap();
            sqlx::query("DELETE FROM workspace_members WHERE workspace_id=$1 AND user_id=$2")
                .bind(workspace)
                .bind(member)
                .execute(&mut *revoke)
                .await
                .unwrap();
            if ban {
                sqlx::query("INSERT INTO workspace_bans (workspace_id,user_id,banned_by,reason) VALUES ($1,$2,$3,'synthetic race')")
                    .bind(workspace).bind(member).bind(owner)
                    .execute(&mut *revoke).await.unwrap();
            }
            let read_pool = pool.clone();
            let read = tokio::spawn(async move {
                if channel_page {
                    own_channel_stats(&read_pool, member, workspace, None, 100)
                        .await
                        .map(|_| ())
                } else {
                    own_workspace_stats(&read_pool, member, workspace)
                        .await
                        .map(|_| ())
                }
            });
            tokio::time::timeout(std::time::Duration::from_secs(5),async {
                loop {
                    let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))").bind(blocker).fetch_one(&pool).await.unwrap();
                    if waiting {break;}
                    assert!(!read.is_finished(),"reader bypassed revocation");
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.expect("reader blocks on real database lock");
            revoke.commit().await.unwrap();
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), read)
                .await
                .unwrap()
                .unwrap();
            let Err(AppError::Denied(reason)) = result else {
                panic!("revocation must deny stats");
            };
            assert_eq!(
                reason,
                if ban {
                    "banned from this workspace"
                } else {
                    "not a workspace member"
                }
            );
        }
        pool.close().await;
    }
}
