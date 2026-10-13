#![cfg(feature = "mls-persist")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tericrypt::mls::{MemoryInstallation, Received};
use tericrypt::mls_persist::{PersistInstallation, PersistProvider, PersistSession, StoreError};

// Synthetic fixtures only: these are not real login passwords.
const PASSWORD: &[u8] = b"synthetic-test-password";
const WRONG_PASSWORD: &[u8] = b"synthetic-wrong-password";

struct TempDb(PathBuf);

impl TempDb {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!(
            "tericrypt-mls-persist-{}-{nanos}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        Self(std::env::temp_dir().join(name))
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = self.0.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn session_survives_provider_drop_and_keeps_delivering_both_ways() {
    let db = TempDb::new();
    let alice =
        PersistInstallation::generate(PersistProvider::open(&db.0, PASSWORD).unwrap()).unwrap();
    let bob = MemoryInstallation::generate().unwrap();
    let mut alice = alice.create_group().unwrap();
    let addition = alice.add(&[bob.key_package().unwrap()]).unwrap();
    let mut bob = bob.join(&addition.welcome).unwrap();

    // A self-update writes the list-shaped own-leaf-node rows.
    let rotation = alice.rotate().unwrap();
    assert_eq!(bob.receive(&rotation).unwrap(), Received::EpochChanged);
    let epoch = alice.epoch();
    drop(alice);

    let mut alice = PersistSession::reopen(PersistProvider::open(&db.0, PASSWORD).unwrap())
        .expect("reopen after full provider drop");
    assert_eq!(alice.epoch(), epoch);
    assert_eq!(alice.member_count(), 2);

    let to_bob = alice.encrypt(b"after restart").unwrap();
    assert_eq!(
        bob.receive(&to_bob).unwrap(),
        Received::Application(b"after restart".to_vec())
    );
    let to_alice = bob.encrypt(b"welcome back").unwrap();
    assert_eq!(
        alice.receive(&to_alice).unwrap(),
        Received::Application(b"welcome back".to_vec())
    );

    // Rotating again after reload reads the persisted own-leaf-node list.
    let rotation = alice.rotate().unwrap();
    assert_eq!(bob.receive(&rotation).unwrap(), Received::EpochChanged);
    drop(alice);
    let alice = PersistSession::reopen(PersistProvider::open(&db.0, PASSWORD).unwrap()).unwrap();
    assert_eq!(alice.epoch(), bob.epoch());
}

#[test]
fn wrong_password_fails_closed() {
    let db = TempDb::new();
    let provider = PersistProvider::open(&db.0, PASSWORD).unwrap();
    let session = PersistInstallation::generate(provider)
        .unwrap()
        .create_group()
        .unwrap();
    drop(session);
    assert_eq!(
        PersistProvider::open(&db.0, WRONG_PASSWORD).unwrap_err(),
        StoreError::Sealed
    );
    PersistSession::reopen(PersistProvider::open(&db.0, PASSWORD).unwrap()).unwrap();
}

#[test]
fn open_inside_async_runtime_returns_typed_error_instead_of_panicking() {
    let db = TempDb::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(async { PersistProvider::open(&db.0, PASSWORD) });
    assert_eq!(result.unwrap_err(), StoreError::SyncContext);
}

#[test]
fn concurrent_first_opens_agree_on_one_salt_and_verifier() {
    for _ in 0..8 {
        let db = TempDb::new();
        let results: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|_| scope.spawn(|| PersistProvider::open(&db.0, PASSWORD).map(|p| p.salt())))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let salts: Vec<_> = results
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        assert!(!salts.is_empty(), "at least one concurrent opener succeeds");
        assert!(salts.windows(2).all(|pair| pair[0] == pair[1]));
        let reopened = PersistProvider::open(&db.0, PASSWORD).expect("file stays openable");
        assert_eq!(reopened.salt(), salts[0]);
    }
}

#[test]
fn installation_survives_restart_between_key_package_and_welcome() {
    let db = TempDb::new();
    let joiner =
        PersistInstallation::generate(PersistProvider::open(&db.0, PASSWORD).unwrap()).unwrap();
    let package = joiner.key_package().unwrap();
    drop(joiner);

    let mut creator = MemoryInstallation::generate()
        .unwrap()
        .create_group()
        .unwrap();
    let addition = creator.add(&[package]).unwrap();

    let joiner =
        PersistInstallation::reopen(PersistProvider::open(&db.0, PASSWORD).unwrap()).unwrap();
    let mut joiner = joiner.join(&addition.welcome).unwrap();
    let to_creator = joiner.encrypt(b"signed by the original leaf").unwrap();
    assert_eq!(
        creator.receive(&to_creator).unwrap(),
        Received::Application(b"signed by the original leaf".to_vec())
    );
}

#[test]
fn provider_opened_on_a_thread_can_be_dropped_inside_an_async_task() {
    let db = TempDb::new();
    let provider = PersistProvider::open(&db.0, PASSWORD).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async move { drop(provider) });
}
