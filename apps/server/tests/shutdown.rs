//! Bounded graceful shutdown: HTTP drain, worker stop, and the real binary
//! exiting cleanly on SIGTERM.
#![forbid(unsafe_code)]

use std::time::Duration;

use axum::routing::get;
use axum::Router;
use terichat_server::{bootstrap, HUB_CAPACITY};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot};

/// Send one `GET` with `Connection: close` and return the raw response.
async fn get_raw(stream: &mut TcpStream, path: &str) -> String {
    let request = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    response
}

fn slow_app() -> Router {
    Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                "done"
            }),
        )
        .route("/hang", get(std::future::pending::<&'static str>))
}

/// An in-flight request finishes after the signal; then the listener is gone.
#[tokio::test]
async fn in_flight_request_drains_before_return() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (fire, fired) = oneshot::channel::<()>();
    let server = tokio::spawn(bootstrap::serve_until(
        listener,
        slow_app(),
        async move {
            let _ = fired.await;
        },
        Duration::from_secs(5),
    ));

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = tokio::spawn(async move { get_raw(&mut stream, "/slow").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    fire.send(()).expect("fire signal");

    let response = request.await.expect("request task");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("done"), "{response}");
    let drained = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("serve_until returns")
        .expect("server task")
        .expect("serve ok");
    assert!(drained, "drain finished before its deadline");
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "listener closed after shutdown"
    );
}

/// A request that never finishes cannot hold shutdown past the deadline.
#[tokio::test]
async fn drain_deadline_bounds_a_hung_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (fire, fired) = oneshot::channel::<()>();
    let server = tokio::spawn(bootstrap::serve_until(
        listener,
        slow_app(),
        async move {
            let _ = fired.await;
        },
        Duration::from_millis(200),
    ));

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let _request = tokio::spawn(async move { get_raw(&mut stream, "/hang").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    fire.send(()).expect("fire signal");

    let drained = tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .expect("serve_until returns at the deadline")
        .expect("server task")
        .expect("serve ok");
    assert!(!drained, "deadline reported as cut short");
}

/// Cooperative workers exit on the signal; one that ignores it is aborted.
#[tokio::test]
async fn workers_stop_or_are_aborted_at_the_deadline() {
    let mut workers = bootstrap::Workers::new();
    workers.spawn("cooperative", |mut stop| async move {
        let _ = stop.wait_for(|stopped| *stopped).await;
    });
    let (dropped_tx, dropped_rx) = oneshot::channel::<()>();
    workers.spawn("stubborn", move |_stop| async move {
        // Signals the test when the abort drops this future.
        let _guard = dropped_tx;
        std::future::pending::<()>().await;
    });

    let started = tokio::time::Instant::now();
    let unclean = workers.shutdown(Duration::from_millis(200)).await;
    assert_eq!(unclean, vec!["stubborn"]);
    assert!(started.elapsed() < Duration::from_secs(2));
    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("aborted worker dropped")
        .expect_err("sender dropped, not sent");
}

/// The real outbox and Stats workers stop cleanly between batches. Requires
/// a live database; skips honestly without one.
#[tokio::test]
async fn database_workers_stop_cleanly() {
    let Some(url) = std::env::var("DATABASE_URL").ok() else {
        eprintln!("SKIPPED: database_workers_stop_cleanly (DATABASE_URL unset)");
        return;
    };
    let pool = terichat_server::db_pool(&url).await.expect("connect");
    terichat_server::MIGRATOR
        .run(&pool)
        .await
        .expect("apply migrations");
    let (hub, _) = broadcast::channel(HUB_CAPACITY);
    let workers = bootstrap::spawn_workers(Some(&pool), &hub);
    // Let both run at least one batch.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let started = tokio::time::Instant::now();
    let unclean = workers.shutdown(Duration::from_secs(5)).await;
    assert!(unclean.is_empty(), "unclean workers: {unclean:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    bootstrap::close_pool(&pool, bootstrap::POOL_CLOSE).await;
    assert!(pool.is_closed());
}

/// SIGTERM makes the real binary drain and exit 0. With `DATABASE_URL` set
/// it also stops both workers and closes the pool.
#[cfg(unix)]
#[tokio::test]
async fn binary_exits_cleanly_on_sigterm() {
    use std::process::Stdio;

    let port = {
        let probe = TcpListener::bind("127.0.0.1:0").await.expect("probe");
        probe.local_addr().expect("addr").port()
    };
    let with_database = std::env::var("DATABASE_URL").is_ok_and(|url| !url.is_empty());
    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_terichat-server"))
        .env("PORT", port.to_string())
        .env("BIND_ADDR", "127.0.0.1")
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn server binary");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await {
            if get_raw(&mut stream, "/health")
                .await
                .starts_with("HTTP/1.1 200")
            {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server never came up"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let pid = child.id().expect("child pid").to_string();
    let killed = tokio::process::Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .await
        .expect("run kill");
    assert!(killed.success());

    let output = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
        .await
        .expect("server exits after SIGTERM")
        .expect("collect output");
    let logs = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "exit {:?}\n{logs}", output.status);
    assert!(logs.contains("shutdown signal received"), "{logs}");
    assert!(logs.contains("terichat-server stopped"), "{logs}");
    if with_database {
        assert_eq!(logs.matches("worker stopped").count(), 2, "{logs}");
        assert!(logs.contains("database pool closed"), "{logs}");
    }
}
