//! Server bootstrap: the startup pieces behind [`main`](crate::main)'s wiring.
//!
//! Database pool plus forward-only migrations, background worker spawn,
//! listener bind, and graceful shutdown. No domain logic lives here.
//!
//! Shutdown drain order, each step bounded:
//!
//! 1. SIGTERM or Ctrl-C stops the listener accepting connections.
//! 2. In-flight HTTP requests finish, up to [`HTTP_DRAIN`]. Workers keep
//!    running meanwhile, so outbox rows those requests write still fan out.
//! 3. Workers are told to stop; each finishes its current batch and exits,
//!    up to [`WORKER_DRAIN`], after which stragglers are aborted.
//! 4. The database pool closes, up to [`POOL_CLOSE`].
//!
//! Nothing claimed is lost at any step: rows not yet marked published are
//! re-claimed by the next boot, and Stats folds are deduped on event id.
//! Upgraded gateway sockets are not HTTP requests, so the drain does not wait
//! for them; they drop when the process exits and clients resume from their
//! cursor.

use std::future::{Future, IntoFuture};
use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::Config;
use crate::messaging;
use crate::stats;

/// Embedded, forward-only `SQLx` migrations from the workspace `migrations/`
/// directory. The macro validates at compile time that the directory exists
/// and parses, so a missing/broken migration fails the build — no database
/// connection required.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Shared database pool for request handlers and boot migration.
///
/// # Errors
///
/// Returns [`sqlx::Error`] when the connection cannot be established.
pub async fn db_pool(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
}

/// Install the process-wide tracing subscriber from configuration.
pub fn init_tracing(config: &Config) {
    fmt()
        .with_env_filter(
            EnvFilter::try_new(&config.rust_log).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();
}

/// Connect the configured database and apply pending migrations.
///
/// Returns [`None`] when no `DATABASE_URL` is set: the server still boots
/// bare with only the probes live.
///
/// # Panics
///
/// Panics when a configured database is unreachable or migrations fail;
/// the server cannot serve then.
pub async fn connect_pool(config: &Config) -> Option<sqlx::PgPool> {
    // Forward-only schema migration, applied once per boot when a database is
    // configured. Without `DATABASE_URL` the server still boots bare.
    match &config.database_url {
        None => None,
        Some(url) => {
            let pool = db_pool(url).await.expect("connect database");
            MIGRATOR
                .run(&pool)
                .await
                .expect("apply pending SQLx migrations");
            tracing::info!("database migrations applied");
            Some(pool)
        }
    }
}

/// Upper bound on waiting for in-flight HTTP requests after the signal.
pub const HTTP_DRAIN: Duration = Duration::from_secs(20);

/// Upper bound on waiting for workers to finish their current batch.
pub const WORKER_DRAIN: Duration = Duration::from_secs(10);

/// Upper bound on returning checked-out database connections.
pub const POOL_CLOSE: Duration = Duration::from_secs(5);

/// Owned background workers: their join handles plus one stop signal.
///
/// Workers receive a [`watch::Receiver`] and return once it reads `true` (or
/// once every sender is gone). Dropping [`Workers`] without calling
/// [`Workers::shutdown`] also signals stop, but does not wait.
pub struct Workers {
    stop: watch::Sender<bool>,
    handles: Vec<(&'static str, JoinHandle<()>)>,
}

impl Default for Workers {
    fn default() -> Self {
        Self::new()
    }
}

impl Workers {
    /// An empty set with its stop signal unset.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stop: watch::channel(false).0,
            handles: Vec::new(),
        }
    }

    /// Spawn one named worker, handing it the shared stop signal.
    pub fn spawn<F, Fut>(&mut self, name: &'static str, worker: F)
    where
        F: FnOnce(watch::Receiver<bool>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::spawn(worker(self.stop.subscribe()));
        self.handles.push((name, handle));
    }

    /// Signal stop and wait for every worker, sharing one `grace` deadline.
    /// Workers still running at the deadline are aborted.
    ///
    /// Returns the names of workers that had to be aborted or that panicked.
    pub async fn shutdown(self, grace: Duration) -> Vec<&'static str> {
        let _ = self.stop.send(true);
        let deadline = tokio::time::Instant::now() + grace;
        let mut unclean = Vec::new();
        for (name, mut handle) in self.handles {
            match tokio::time::timeout_at(deadline, &mut handle).await {
                Ok(Ok(())) => tracing::info!(worker = name, "worker stopped"),
                Ok(Err(err)) => {
                    tracing::error!(worker = name, "worker failed: {err}");
                    unclean.push(name);
                }
                Err(_) => {
                    tracing::warn!(worker = name, "worker drain deadline elapsed; aborting");
                    handle.abort();
                    unclean.push(name);
                }
            }
        }
        unclean
    }
}

/// Spawn the background workers when a database is configured.
///
/// At-least-once fan-out; the next boot re-claims anything unmarked.
#[must_use]
pub fn spawn_workers(
    pool: Option<&sqlx::PgPool>,
    hub: &broadcast::Sender<messaging::OutboxEntry>,
) -> Workers {
    let mut workers = Workers::new();
    if let Some(pool) = pool {
        let (outbox_pool, hub) = (pool.clone(), hub.clone());
        workers.spawn("outbox", move |stop| {
            messaging::outbox_worker(outbox_pool, hub, stop)
        });
        // Stats fold: at-least-once too, deduped on the outbox event id.
        let stats_pool = pool.clone();
        workers.spawn("stats", move |stop| stats::stats_worker(stats_pool, stop));
    }
    workers
}

/// Bind the TCP listener for the configured address.
///
/// # Panics
///
/// Panics when the address cannot be bound; the server cannot serve then.
pub async fn bind_listener(addr: SocketAddr) -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind server address")
}

/// Resolve on SIGTERM (Unix) or Ctrl-C, whichever comes first.
///
/// A handler that cannot be installed is logged and never resolves, so a
/// failed install cannot shut the server down on its own.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!("cannot listen for Ctrl-C: {err}");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::error!("cannot listen for SIGTERM: {err}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received");
}

/// Serve `app` until `signal` resolves, then drain in-flight requests for at
/// most `drain`. The listener stops accepting as soon as the signal fires.
///
/// Returns `Ok(true)` when every request finished, `Ok(false)` when the
/// deadline cut the drain short (remaining connections are dropped).
///
/// # Errors
///
/// Returns the I/O error that stopped the accept loop.
pub async fn serve_until(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    signal: impl Future<Output = ()> + Send + 'static,
    drain: Duration,
) -> std::io::Result<bool> {
    let (signalled_tx, mut signalled) = watch::channel(false);
    let serve = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            signal.await;
            let _ = signalled_tx.send(true);
        })
        .into_future();
    tokio::pin!(serve);
    tokio::select! {
        served = &mut serve => served.map(|()| true),
        () = async {
            // Err means the serve future (and its sender) is gone; the arm
            // above is then already complete.
            let _ = signalled.wait_for(|fired| *fired).await;
            tokio::time::sleep(drain).await;
        } => {
            tracing::warn!("HTTP drain deadline elapsed; dropping open connections");
            Ok(false)
        }
    }
}

/// Close the pool, waiting at most `grace` for checked-out connections.
pub async fn close_pool(pool: &sqlx::PgPool, grace: Duration) {
    if tokio::time::timeout(grace, pool.close()).await.is_err() {
        tracing::warn!("database pool close deadline elapsed");
    } else {
        tracing::info!("database pool closed");
    }
}
