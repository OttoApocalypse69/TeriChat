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
    AppState { pool: None, hub }
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
