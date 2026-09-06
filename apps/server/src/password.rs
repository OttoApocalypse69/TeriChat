//! Argon2id password hashing for the upcoming auth slice.
//!
//! [`hash_password`] hashes with [`Argon2::default()`] and a fresh random
//! salt per call, returning the PHC-encoded string for storage.
//! [`verify_password`] checks a candidate against a stored PHC hash and
//! returns `false` — never panics — on malformed input or mismatch.
//!
//! Password hashes are secrets: callers must never log them.

use std::fmt;

use argon2::Argon2;
use password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand_core::OsRng;

/// Typed error for password hashing failures.
#[derive(Debug)]
pub enum PasswordError {
    /// Argon2 hashing failed (invalid parameters, RNG or encoding failure).
    HashFailed(password_hash::Error),
}

impl fmt::Display for PasswordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HashFailed(err) => write!(f, "password hashing failed: {err}"),
        }
    }
}

impl std::error::Error for PasswordError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HashFailed(err) => Some(err),
        }
    }
}

/// Hash `password` with Argon2id, returning the PHC-encoded hash string.
///
/// # Why `Argon2::default()`
///
/// The crate default is the OWASP-recommended Argon2id baseline (19 MiB
/// memory, 2 iterations, 1 lane). Accepting it instead of inventing custom
/// parameters keeps the choice auditable and avoids silently weakening the
/// memory-hardness guarantee; tuning, if ever needed, belongs in an ADR, not
/// in a magic constant here.
///
/// # Errors
///
/// Returns [`PasswordError`] if hashing fails (RNG, parameter, or encoding
/// failure inside the `argon2`/`password-hash` crates).
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(PasswordError::HashFailed)
}

/// Verify `password` against a PHC-encoded Argon2 hash string.
///
/// Returns `false` — never panics — when `hash` is malformed or the password
/// does not match.
#[must_use]
pub fn verify_password(hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip_succeeds() {
        let hash = hash_password("correct horse battery staple").expect("hash password");
        assert!(verify_password(&hash, "correct horse battery staple"));
    }

    #[test]
    fn wrong_password_returns_false() {
        let hash = hash_password("right-password").expect("hash password");
        assert!(!verify_password(&hash, "wrong-password"));
    }

    #[test]
    fn malformed_hash_string_returns_false() {
        assert!(!verify_password("not-a-valid-phc-hash", "any-password"));
        assert!(!verify_password("", "any-password"));
    }

    #[test]
    fn empty_password_roundtrips() {
        // Argon2 accepts empty input; pin the behavior so a future length
        // floor cannot silently lock users out.
        let hash = hash_password("").expect("hash password");
        assert!(verify_password(&hash, ""));
        assert!(!verify_password(&hash, "x"));
    }

    #[test]
    fn same_password_produces_different_hashes() {
        let first = hash_password("same-password").expect("hash password");
        let second = hash_password("same-password").expect("hash password");
        assert_ne!(first, second);
    }

    #[test]
    fn phc_output_uses_argon2id() {
        let hash = hash_password("any-password").expect("hash password");
        assert!(hash.starts_with("$argon2id$"));
    }
}
