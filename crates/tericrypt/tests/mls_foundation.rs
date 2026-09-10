#![cfg(feature = "mls-foundation")]

use tericrypt::mls::{MemoryInstallation, Received, Session, SUITE};

fn four_devices() -> Vec<Session> {
    // Account labels exist only in this harness, never in the MLS credential.
    let alice_desktop = MemoryInstallation::generate().expect("alice desktop");
    let others: Vec<_> = ["alice phone", "bob desktop", "bob phone"]
        .into_iter()
        .map(|_| MemoryInstallation::generate().expect("independent installation"))
        .collect();
    let packages: Vec<_> = others.iter().map(|d| d.key_package().unwrap()).collect();
    let mut creator = alice_desktop.create_group().unwrap();
    let addition = creator.add(&packages).unwrap();
    let mut sessions = vec![creator];
    sessions.extend(
        others
            .into_iter()
            .map(|d| d.join(&addition.welcome).unwrap()),
    );
    sessions
}

#[test]
fn removed_installation_cannot_decrypt_future_epoch_even_without_removal_notice() {
    let mut sessions = four_devices();
    let removed_identity = sessions[3].scoped_identity();
    let before = sessions[0].encrypt(b"before removal").unwrap();
    assert_eq!(
        sessions[3].receive(&before).unwrap(),
        Received::Application(b"before removal".to_vec())
    );
    let commit = sessions[0].remove(&removed_identity).unwrap();
    assert_eq!(sessions[0].epoch(), 2);
    for session in sessions.iter_mut().take(3).skip(1) {
        assert_eq!(session.receive(&commit).unwrap(), Received::EpochChanged);
        assert_eq!(session.epoch(), 2);
        assert_eq!(session.member_count(), 3);
    }
    let future = sessions[0].encrypt(b"after removal").unwrap();
    assert!(
        sessions[3].receive(&future).is_err(),
        "old keys cannot decrypt new epoch, even when notice withheld"
    );
    assert_eq!(
        sessions[3].receive(&commit).unwrap(),
        Received::EpochChanged
    );
    assert!(sessions[3].receive(&future).is_err());
    assert!(sessions[3].encrypt(b"removed sender").is_err());
    for session in sessions.iter_mut().take(3).skip(1) {
        assert_eq!(
            session.receive(&future).unwrap(),
            Received::Application(b"after removal".to_vec())
        );
    }
}

#[test]
fn adding_installation_advances_existing_peers_epoch() {
    let mut sessions = four_devices();
    let new_device = MemoryInstallation::generate().unwrap();
    let addition = sessions[0]
        .add(&[new_device.key_package().unwrap()])
        .unwrap();
    assert_eq!(sessions[0].epoch(), 2);
    for session in sessions.iter_mut().skip(1) {
        session
            .receive(&addition.commit)
            .expect("merge valid add commit");
        assert_eq!(session.epoch(), 2);
        assert_eq!(session.member_count(), 5);
    }
    sessions.push(new_device.join(&addition.welcome).unwrap());
    assert_eq!(sessions[4].epoch(), 2);
    let wire = sessions[4].encrypt(b"new installation").unwrap();
    for session in sessions.iter_mut().take(4) {
        assert_eq!(
            session.receive(&wire).unwrap(),
            Received::Application(b"new installation".to_vec())
        );
    }
}

#[test]
fn two_users_four_independent_installations_exchange() {
    let mut sessions = four_devices();
    for session in &sessions {
        assert_eq!(session.member_count(), 4);
        assert_eq!(session.ciphersuite(), SUITE);
        assert_eq!(session.epoch(), 1);
    }
    let mut keys: Vec<_> = sessions.iter().map(Session::signature_public_key).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 4);
    let mut credentials: Vec<_> = sessions.iter().map(Session::scoped_identity).collect();
    credentials.sort();
    credentials.dedup();
    assert_eq!(credentials.len(), 4);
    for sender in 0..sessions.len() {
        let plaintext = format!("synthetic message from installation {sender}").into_bytes();
        let wire = sessions[sender].encrypt(&plaintext).unwrap();
        assert!(!wire.windows(plaintext.len()).any(|w| w == plaintext));
        for (recipient, session) in sessions.iter_mut().enumerate() {
            if recipient != sender {
                assert_eq!(
                    session.receive(&wire).unwrap(),
                    Received::Application(plaintext.clone())
                );
            }
        }
    }
}
