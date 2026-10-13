#![cfg(feature = "mls-foundation")]

use tericrypt::mls::{MemoryInstallation, MlsError, Received, Session};

fn pair() -> (Session, Session) {
    let mut alice = MemoryInstallation::generate()
        .unwrap()
        .create_group()
        .unwrap();
    let bob = MemoryInstallation::generate().unwrap();
    let addition = alice.add(&[bob.key_package().unwrap()]).unwrap();
    (alice, bob.join(&addition.welcome).unwrap())
}

#[test]
fn malformed_trailing_tampered_and_replayed_messages_fail_closed() {
    let (mut alice, mut bob) = pair();
    for malformed in [vec![], vec![0], vec![255; 16]] {
        assert!(bob.receive(&malformed).is_err());
    }
    let wire = alice.encrypt(b"synthetic valid message").unwrap();
    let mut trailing = wire.clone();
    trailing.push(0);
    assert_eq!(bob.receive(&trailing), Err(MlsError::InvalidInput));
    let mut tampered = wire.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(bob.receive(&tampered).is_err());
    assert_eq!(bob.epoch(), 1);
    assert_eq!(
        bob.receive(&wire).unwrap(),
        Received::Application(b"synthetic valid message".to_vec())
    );
    assert!(bob.receive(&wire).is_err());
}

#[test]
fn foreign_group_application_and_commit_do_not_change_target_epoch() {
    let (mut alice, mut bob) = pair();
    let (mut foreign, _) = pair();
    let wire = foreign.encrypt(b"other group").unwrap();
    assert!(bob.receive(&wire).is_err());
    let newcomer = MemoryInstallation::generate().unwrap();
    let addition = foreign.add(&[newcomer.key_package().unwrap()]).unwrap();
    assert!(bob.receive(&addition.commit).is_err());
    assert!(bob.receive(&addition.welcome).is_err());
    assert_eq!(bob.epoch(), 1);
    let legitimate = alice.encrypt(b"still works").unwrap();
    assert_eq!(
        bob.receive(&legitimate).unwrap(),
        Received::Application(b"still works".to_vec())
    );
}

#[test]
fn malformed_packages_and_foreign_welcome_are_rejected() {
    let (mut alice, _) = pair();
    assert!(alice.add(&[vec![0; 32]]).is_err());
    assert!(alice.remove(b"not a member").is_err());
    assert_eq!(alice.epoch(), 1);
    let bob = MemoryInstallation::generate().unwrap();
    let mut package = bob.key_package().unwrap();
    package.push(0);
    assert!(alice.add(&[package]).is_err());
    let addition = alice.add(&[bob.key_package().unwrap()]).unwrap();
    let stranger = MemoryInstallation::generate().unwrap();
    assert!(stranger.join(&addition.welcome).is_err());
    let malformed = MemoryInstallation::generate().unwrap();
    assert!(matches!(
        malformed.join(&[0; 12]),
        Err(MlsError::InvalidInput)
    ));
    let wrong_kind = MemoryInstallation::generate().unwrap();
    assert!(matches!(
        wrong_kind.join(&addition.commit),
        Err(MlsError::InvalidInput)
    ));
}

#[test]
fn poc_envelope_is_not_an_mls_message_and_never_downgrades() {
    let (_, mut bob) = pair();
    let sender = tericrypt::IdentityKeypair::generate().unwrap();
    let recipient = tericrypt::IdentityKeypair::generate().unwrap();
    let wire = tericrypt::seal(&sender, &recipient.agreement_pubkey(), b"legacy synthetic")
        .unwrap()
        .to_bytes();
    assert!(bob.receive(&wire).is_err());
}

#[test]
fn valid_alternate_suite_packages_and_welcome_are_rejected() {
    use openmls::prelude::{tls_codec::Serialize, *};
    use openmls_basic_credential::SignatureKeyPair;
    use openmls_rust_crypto::OpenMlsRustCrypto;

    let alternate = Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519;
    let provider = OpenMlsRustCrypto::default();
    let signer = SignatureKeyPair::new(alternate.signature_algorithm()).unwrap();
    let credential = CredentialWithKey {
        credential: BasicCredential::new(b"synthetic alternate-suite device".to_vec()).into(),
        signature_key: signer.to_public_vec().into(),
    };
    let package = KeyPackage::builder()
        .build(alternate, &provider, &signer, credential.clone())
        .unwrap();
    let (mut alice, _) = pair();
    assert!(matches!(
        alice.add(&[package.key_package().tls_serialize_detached().unwrap()]),
        Err(MlsError::UnsupportedSuite)
    ));
    assert_eq!(alice.epoch(), 1);

    let creator_provider = OpenMlsRustCrypto::default();
    let creator_signer = SignatureKeyPair::new(alternate.signature_algorithm()).unwrap();
    let creator_credential = CredentialWithKey {
        credential: BasicCredential::new(b"synthetic alternate-suite creator".to_vec()).into(),
        signature_key: creator_signer.to_public_vec().into(),
    };
    let config = MlsGroupCreateConfig::builder()
        .ciphersuite(alternate)
        .use_ratchet_tree_extension(true)
        .build();
    let mut group = MlsGroup::new(
        &creator_provider,
        &creator_signer,
        &config,
        creator_credential,
    )
    .unwrap();
    let (_, welcome, _) = group
        .add_members(
            &creator_provider,
            &creator_signer,
            &[package.key_package().clone()],
        )
        .unwrap();
    assert!(matches!(
        MemoryInstallation::generate()
            .unwrap()
            .join(&welcome.to_bytes().unwrap()),
        Err(MlsError::UnsupportedSuite)
    ));
}

#[test]
fn failed_receive_reconciliation_preserves_send_and_replay_state() {
    let (mut alice, mut bob) = pair();
    let first = alice.encrypt(b"first").unwrap();
    assert_eq!(
        bob.receive(&first).unwrap(),
        Received::Application(b"first".to_vec())
    );
    let reply = bob.encrypt(b"reply").unwrap();
    let mut corrupted = reply.clone();
    *corrupted.last_mut().unwrap() ^= 1;
    assert!(alice.receive(&corrupted).is_err());
    let second = alice.encrypt(b"second").unwrap();
    assert_eq!(
        bob.receive(&second).unwrap(),
        Received::Application(b"second".to_vec())
    );
    assert_eq!(
        alice.receive(&reply).unwrap(),
        Received::Application(b"reply".to_vec())
    );
    for _ in 0..2 {
        assert!(bob.receive(&first).is_err());
        assert!(bob.receive(&second).is_err());
        assert!(alice.receive(&reply).is_err());
    }
}

#[test]
fn oversized_plaintext_is_rejected_before_advancing_sender_state() {
    let (mut alice, mut bob) = pair();
    assert!(alice.encrypt(&vec![0; 65_537]).is_err());
    let wire = alice.encrypt(b"still usable").unwrap();
    assert_eq!(
        bob.receive(&wire).unwrap(),
        Received::Application(b"still usable".to_vec())
    );
}
