//! Issue #11 acceptance tests: 256-bit recovery secret as 24 BIP-39 words.
//!
//! Synthetic fixtures only. No real recovery material is logged or asserted.

use tericrypt::recovery;

/// Official BIP-39 256-bit vector: 32 zero bytes <-> "abandon" x23 + "art".
fn zero_phrase() -> String {
    std::iter::repeat_n("abandon", 23)
        .chain(std::iter::once("art"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn zero_entropy_matches_official_bip39_vector() {
    let phrase = recovery::encode(&[0u8; 32]);
    assert_eq!(phrase, zero_phrase(), "must match official BIP-39 vector");
    let entropy = recovery::decode(&zero_phrase()).expect("valid vector decodes");
    assert_eq!(&*entropy, &[0u8; 32]);
}

#[test]
fn roundtrip_is_deterministic_over_24_words() {
    let raw: [u8; 32] =
        std::array::from_fn(|i| u8::try_from(i).unwrap().wrapping_mul(7).wrapping_add(3));
    let first = recovery::encode(&raw);
    let second = recovery::encode(&raw);
    assert_eq!(first, second, "encoding must be deterministic");
    assert_eq!(first.split_whitespace().count(), 24);
    let back = recovery::decode(&first).expect("roundtrip decodes");
    assert_eq!(&*back, &raw);
}

/// One corrupt word is located, not merely rejected.
#[test]
fn corrupt_word_reports_its_position() {
    let good = zero_phrase();
    let mut words: Vec<&str> = good.split_whitespace().collect();
    words[5] = "notaword";
    let bad = words.join(" ");
    let err = recovery::decode(&bad).expect_err("unknown word must fail");
    assert_eq!(err, recovery::RecoveryError::BadWord { index: 5 });
}

/// Same words with a broken checksum fail closed before any vault use.
#[test]
fn checksum_tamper_is_rejected() {
    let good = zero_phrase();
    let tampered = good.rsplit_once(' ').unwrap().0.to_owned() + " abandon";
    assert_ne!(tampered, good);
    let err = recovery::decode(&tampered).expect_err("bad checksum must fail");
    assert_eq!(err, recovery::RecoveryError::BadChecksum);
}

/// Official 12-word vector is a different length class: rejected as length,
/// never silently upgraded or reinterpreted.
#[test]
fn twelve_word_phrase_is_wrong_length_class() {
    let twelve = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let err = recovery::decode(twelve).expect_err("12 words must fail");
    assert_eq!(err, recovery::RecoveryError::BadLength { words: 12 });
}

/// Pasted phrases with case/whitespace noise still recover.
#[test]
fn noisy_paste_normalizes_and_decodes() {
    let noisy = "  ABANDON\tAbandon\nabandon  abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon ART  ";
    let entropy = recovery::decode(noisy).expect("noisy paste must decode");
    assert_eq!(&*entropy, &[0u8; 32]);
}

/// Fullwidth input folds through NFKD to the canonical phrase.
#[test]
fn fullwidth_input_folds_through_nfkd() {
    let wide: String = "ａｂａｎｄｏｎ ".repeat(23) + "ａｒｔ";
    let entropy = recovery::decode(&wide).expect("NFKD input must decode");
    assert_eq!(&*entropy, &[0u8; 32]);
}

/// Canonical form passes the canonical check; noisy input does not.
#[test]
fn canonical_check_flags_non_canonical_only() {
    assert_eq!(recovery::check_canonical(&zero_phrase()), Ok(()));
    assert_eq!(
        recovery::check_canonical("ABANDON abandon"),
        Err(recovery::RecoveryError::NotNormalized)
    );
}

/// Error text carries positions and counts only: no words, no entropy.
#[test]
fn errors_echo_no_secret_material() {
    let good = zero_phrase();
    let secret_words: Vec<&str> = good.split_whitespace().collect();
    let errors = [
        recovery::decode("short phrase").expect_err("length"),
        recovery::decode(&secret_words.join(" ").replace("abandon", "notaword")).expect_err("word"),
        recovery::decode(&zero_phrase().replace(" art", " abandon")).expect_err("checksum"),
        recovery::check_canonical("LOUD NOISE").expect_err("canonical"),
    ];
    for err in errors {
        for rendered in [format!("{err}"), format!("{err:?}")] {
            for word in &secret_words {
                assert!(!rendered.contains(word), "error leaks a word: {rendered}");
            }
            assert!(!rendered.contains("art"), "error leaks checksum word");
        }
    }
}
