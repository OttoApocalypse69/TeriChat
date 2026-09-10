//! Migration embedding and apply checks.
//!
//! Moved verbatim from `src/main.rs` integration module.
#![forbid(unsafe_code)]
use terichat_server::{db_pool, MIGRATOR};

#[test]
fn migrator_embeds_identity_migration() {
    // Offline: the embedded migrator must resolve the known migrations.
    // The `migrate!` macro already fails the build when the directory is
    // missing or unparsable; this pins the expected content.
    assert!(
        !MIGRATOR.migrations.is_empty(),
        "expected at least the identity migration"
    );
    // Note: sqlx renders filename underscores as spaces in descriptions.
    for expected in ["identity", "messaging", "device agreement keys"] {
        assert!(
            MIGRATOR
                .migrations
                .iter()
                .any(|migration| migration.description.contains(expected)),
            "expected a {expected} migration"
        );
    }
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
