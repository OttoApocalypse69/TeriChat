//! `TeriChat` modular-monolith API server (Alpha 0 baseline).
//!
//! Serves [`/health`](health) liveness and [`/ready`](ready) readiness.
//! Readiness reports the database as `not_configured` when `DATABASE_URL`
//! is unset so the baseline boots without external services; `Compose`
//! provides the real `PostgreSQL` for the next slice.

#![forbid(unsafe_code)]

mod config;
mod password;

use std::{net::SocketAddr, time::Duration};

use axum::{http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use tracing_subscriber::{fmt, EnvFilter};

/// Shared JSON error shape for the API baseline.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

/// Machine-readable error detail.
#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

/// Minimal typed application error.
#[derive(Debug)]
enum AppError {
    /// Readiness dependency unavailable.
    Unavailable(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", message),
        };
        let body = Json(ErrorBody {
            error: ErrorDetail { code, message },
        });
        (status, body).into_response()
    }
}

/// Liveness probe — never touches dependencies.
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

/// Readiness probe — reports database wiring state.
#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    database: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "terichat-server",
    })
}

async fn ready() -> Result<Json<ReadyResponse>, AppError> {
    ready_from_env(std::env::var("DATABASE_URL").ok()).await
}

/// Readiness core, separated from process-env access so tests can inject
/// the database configuration directly without mutating shared state.
async fn ready_from_env(database_url: Option<String>) -> Result<Json<ReadyResponse>, AppError> {
    match database_url {
        None => Ok(Json(ReadyResponse {
            status: "ok",
            database: "not_configured",
        })),
        Some(url) => {
            let pool = db_pool(&url)
                .await
                .map_err(|err| AppError::Unavailable(format!("database connect failed: {err}")))?;
            sqlx::query("SELECT 1")
                .execute(&pool)
                .await
                .map_err(|err| AppError::Unavailable(format!("database ping failed: {err}")))?;
            pool.close().await;
            Ok(Json(ReadyResponse {
                status: "ok",
                database: "connected",
            }))
        }
    }
}

/// Embedded, forward-only `SQLx` migrations from the workspace `migrations/`
/// directory. The macro validates at compile time that the directory exists
/// and parses, so a missing/broken migration fails the build — no database
/// connection required.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Single-connection pool helper shared by readiness and boot migration.
async fn db_pool(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(2))
        .connect(url)
        .await
}

fn router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
}

fn bind_addr() -> SocketAddr {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(3001);
    SocketAddr::from(([127, 0, 0, 1], port))
}

#[tokio::main]
async fn main() {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let addr = bind_addr();

    // Forward-only schema migration, applied once per boot when a database is
    // configured. Without `DATABASE_URL` the baseline still boots bare.
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pool = db_pool(&url)
            .await
            .expect("connect database for migrations");
        MIGRATOR
            .run(&pool)
            .await
            .expect("apply pending SQLx migrations");
        pool.close().await;
        tracing::info!("database migrations applied");
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind server address");
    tracing::info!(%addr, "terichat-server listening");
    axum::serve(listener, router())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve axum router");
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

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

    #[tokio::test]
    async fn health_returns_ok() {
        let (status, json) = body_json(router(), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "terichat-server");
    }

    #[tokio::test]
    async fn ready_without_database_url_reports_not_configured() {
        let response = ready_from_env(None).await.unwrap().0;
        let body = serde_json::to_value(&response).unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["database"], "not_configured");
    }

    #[tokio::test]
    async fn ready_with_unreachable_database_reports_unavailable() {
        let err = ready_from_env(Some("postgres://127.0.0.1:1/terichat_test".to_string()))
            .await
            .unwrap_err();
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn ready_route_serves_without_database() {
        // Route-level smoke test for the no-database boot path. Precondition:
        // no `DATABASE_URL` in this process — with one set (e.g. the DB-backed
        // suite or the new CI db-tests job) the handler legitimately reports
        // `connected`, so skip honestly instead of asserting the wrong state.
        if std::env::var("DATABASE_URL").is_ok() {
            eprintln!("SKIPPED: ready_route_serves_without_database (DATABASE_URL is set)");
            return;
        }
        let (status, json) = body_json(router(), "/ready").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
        assert_eq!(json["database"], "not_configured");
    }

    #[test]
    fn migrator_embeds_identity_migration() {
        // Offline: the embedded migrator must resolve at least the identity
        // migration. The `migrate!` macro already fails the build when the
        // directory is missing or unparsable; this pins the expected content.
        assert!(
            !MIGRATOR.migrations.is_empty(),
            "expected at least the identity migration"
        );
        let latest = MIGRATOR
            .migrations
            .iter()
            .max_by_key(|m| m.version)
            .unwrap();
        assert!(
            latest.description.contains("identity"),
            "latest migration should be the identity foundation"
        );
    }

    #[tokio::test]
    async fn migrations_apply_and_create_identity_tables() {
        // Requires a live database: `DATABASE_URL=... cargo test`. Prints a
        // skip (never a fake pass) when no database is configured, so plain
        // `cargo test` stays offline-clean for CI without services.
        let Some(url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("SKIPPED: migrations_apply_and_create_identity_tables (DATABASE_URL unset)");
            return;
        };
        let pool = db_pool(&url).await.expect("connect test database");
        MIGRATOR.run(&pool).await.expect("apply migrations");

        for table in ["users", "devices", "sessions"] {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("query information_schema");
            assert!(exists, "expected table `{table}` after migrations");
        }
        pool.close().await;
    }
}
