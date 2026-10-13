//! Probes and no-database gates over the real router.
//!
//! Moved verbatim from `src/main.rs` integration module: behavior-preserving,
//! no API changes.
#![forbid(unsafe_code)]
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use terichat_server::{build_router, db_pool, AppState, HUB_CAPACITY};
use tokio::sync::broadcast;
use tower::ServiceExt;

fn bare_state() -> AppState {
    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    AppState::new(
        None,
        hub,
        std::env::temp_dir().join("terichat-test-attachments"),
    )
}

async fn body_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (status, json)
}

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

#[tokio::test]
async fn health_returns_ok() {
    let (status, json) = body_json(build_router(bare_state()), "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "terichat-server");
}

#[tokio::test]
async fn ready_without_database_reports_not_configured() {
    let (status, json) = body_json(build_router(bare_state()), "/ready").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
    assert_eq!(json["database"], "not_configured");
}

#[tokio::test]
async fn ready_with_unreachable_database_is_down() {
    let pool = db_pool("postgres://127.0.0.1:1/terichat_test").await;
    assert!(pool.is_err(), "expected connect failure, got {pool:?}");
}

fn pooled_app(pool: sqlx::PgPool) -> Router {
    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    build_router(AppState::new(
        Some(pool),
        hub,
        std::env::temp_dir().join("terichat-test-attachments"),
    ))
}

fn assert_unavailable(status: StatusCode, json: &serde_json::Value) {
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        json,
        &serde_json::json!({
            "error": { "code": "unavailable", "message": "database unavailable" }
        })
    );
}

#[tokio::test]
async fn ready_closed_pool_returns_safe_error() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/synthetic_readiness")
        .unwrap();
    pool.close().await;
    let (status, json) = body_json(pooled_app(pool), "/ready").await;
    assert_unavailable(status, &json);
}

#[tokio::test]
async fn ready_stalled_database_has_overall_deadline() {
    use std::time::Duration;

    // A real loopback peer accepts TCP but deliberately never answers the
    // PostgreSQL handshake. No database service or credentials are involved.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(20))
        .connect_lazy(&format!(
            "postgres://{address}/synthetic_readiness?sslmode=disable"
        ))
        .unwrap();
    let app = pooled_app(pool.clone());
    let (health_status, _) = body_json(app.clone(), "/health").await;
    assert_eq!(health_status, StatusCode::OK);
    let outcome = tokio::time::timeout(Duration::from_secs(4), body_json(app, "/ready")).await;
    peer.abort();
    let _ = peer.await;
    tokio::time::timeout(Duration::from_secs(1), pool.close())
        .await
        .unwrap();
    let (status, json) = outcome.expect("readiness must finish before the outer watchdog");
    assert_unavailable(status, &json);
}

#[sqlx::test]
async fn ready_recovers_after_pool_saturation(pool: sqlx::PgPool) {
    use std::time::Duration;

    let probe_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(20))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let lease = probe_pool.acquire().await.unwrap();
    let app = pooled_app(probe_pool.clone());
    let outcome =
        tokio::time::timeout(Duration::from_secs(4), body_json(app.clone(), "/ready")).await;
    drop(lease);
    let (status, json) = outcome.expect("exhausted pool must respect readiness deadline");
    assert_unavailable(status, &json);
    let (status, json) = tokio::time::timeout(Duration::from_secs(4), body_json(app, "/ready"))
        .await
        .expect("readiness must recover after releasing the lease");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json,
        serde_json::json!({"status": "ok", "database": "connected"})
    );
    probe_pool.close().await;
}

#[tokio::test]
async fn auth_routes_need_a_database() {
    // Probes-only boot: every account route reports 503, never 500.
    let app = build_router(bare_state());
    let (status, json) = post_json(
        app,
        "/v1/auth/register",
        serde_json::json!({"handle":"x","email":"x@y.z","display_name":"X","password":"pw"}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["error"]["code"], "no_database");
}

#[tokio::test]
async fn bearer_rejection_shapes() {
    // No database and no token alike: 401/503 JSON, never a bare status.
    let (status, json) = post_json(
        build_router(bare_state()),
        "/v1/auth/logout",
        serde_json::json!({}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["error"]["code"], "no_database");
}
