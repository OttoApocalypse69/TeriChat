//! `terirecovery-v1`: 256-bit recovery secret as 24 BIP-39 English words.
//!
//! Format policy: version 1 is exactly BIP-39 English over 32 bytes of
//! entropy ([`FORMAT_VERSION`]). A future version gets a new constant and its
//! own parsing entry point; v1 phrases keep decoding as v1, and shorter
//! length classes (e.g. 12 words) are rejected, never silently upgraded.
//!
//! Handling policy: recovery words and entropy are client-only. They are
//! never sent to the server, and neither this module nor its errors log,
//! print, or echo them (see `Display`).

use std::borrow::Cow;

use bip39::{Language, Mnemonic};
use zeroize::Zeroizing;

/// Words per recovery phrase. Fixed: 32 bytes of entropy plus its 8-bit
/// checksum encode to exactly 24 words.
pub const WORD_COUNT: usize = 24;

/// Entropy bytes per recovery secret (256-bit, machine-generated).
pub const ENTROPY_BYTES: usize = 32;

/// Recovery format version implemented here (`terirecovery-v1`).
pub const FORMAT_VERSION: u32 = 1;

/// Typed recovery-phrase error. Messages carry counts and positions only:
/// never a word, never entropy bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryError {
    /// Wrong word count (e.g. a 12-word phrase: a different length class).
    BadLength {
        /// Words found in the input.
        words: usize,
    },
    /// Word at `index` (0-based) is not in the BIP-39 English wordlist.
    BadWord {
        /// 0-based position of the unknown word.
        index: usize,
    },
    /// Words parse but the checksum does not match the entropy.
    BadChecksum,
    /// Input is not canonical (lowercase, single spaces, NFKD).
    NotNormalized,
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadLength { words } => write!(
                f,
                "recovery phrase has wrong length: found {words} words, expected {WORD_COUNT}"
            ),
            Self::BadWord { index } => {
                write!(
                    f,
                    "recovery phrase has an unrecognized word at index {index}"
                )
            }
            Self::BadChecksum => write!(f, "recovery phrase failed checksum validation"),
            Self::NotNormalized => write!(
                f,
                "recovery phrase is not in canonical form (lowercase, single spaces, NFKD)"
            ),
        }
    }
}

impl std::error::Error for RecoveryError {}

/// Encode 32 bytes of entropy as 24 BIP-39 English words (single spaces).
///
/// # Panics
///
/// Never panics on 32-byte input: that size is always valid BIP-39 entropy.
/// The internal assertion documents the invariant rather than handling a
/// case that cannot occur.
#[must_use]
pub fn encode(entropy: &[u8; 32]) -> String {
    Mnemonic::from_entropy_in(Language::English, entropy)
        .expect("32-byte entropy is always valid BIP-39 entropy")
        .to_string()
}

/// Decode a phrase to its 32 entropy bytes, accepting case, whitespace, and
/// Unicode (NFKD) variants of the canonical form.
///
/// # Errors
///
/// Returns [`RecoveryError`] when the phrase has the wrong shape or fails
/// checksum validation. Never returns words or entropy bytes in the error.
pub fn decode(phrase: &str) -> Result<Zeroizing<[u8; 32]>, RecoveryError> {
    parse(&normalize(phrase))
}

/// NFKD-normalize (per the vendored `bip39` helper), lowercase, and collapse
/// all whitespace to single spaces.
fn normalize(phrase: &str) -> String {
    let mut cow = Cow::Borrowed(phrase);
    Mnemonic::normalize_utf8_cow(&mut cow);
    cow.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns `Ok(())` when `phrase` is already canonical (lowercase, single
/// spaces, NFKD). Recovery entry fields accept variants via [`decode`];
/// use this to warn when a stored or pasted phrase is not canonical.
/// Never echoes the phrase, only its shape.
///
/// # Errors
///
/// Returns [`RecoveryError::NotNormalized`] when the input is not already
/// canonical. The error carries no words or entropy.
pub fn check_canonical(phrase: &str) -> Result<(), RecoveryError> {
    if normalize(phrase) == phrase {
        Ok(())
    } else {
        Err(RecoveryError::NotNormalized)
    }
}

/// Parse an already-normalized phrase.
fn parse(canonical: &str) -> Result<Zeroizing<[u8; 32]>, RecoveryError> {
    let words = canonical.split_whitespace().count();
    if words != WORD_COUNT {
        return Err(RecoveryError::BadLength { words });
    }
    let mnemonic =
        Mnemonic::parse_in_normalized(Language::English, canonical).map_err(|err| match err {
            bip39::Error::BadWordCount(words) => RecoveryError::BadLength { words },
            bip39::Error::UnknownWord(index) => RecoveryError::BadWord { index },
            _ => RecoveryError::BadChecksum,
        })?;
    let entropy = mnemonic.to_entropy();
    if entropy.len() != ENTROPY_BYTES {
        return Err(RecoveryError::BadChecksum);
    }
    let mut out = Zeroizing::new([0_u8; 32]);
    out.copy_from_slice(&entropy);
    Ok(out)
}
