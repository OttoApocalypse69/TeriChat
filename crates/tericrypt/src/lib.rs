//! `TeriCrypt-4096` device cryptography — Alpha proof of concept.
//!
//! Each installation owns an [`IdentityKeypair`]: an `Ed25519` signing key
//! (who the device claims to be) and an `X25519` agreement key (how others
//! reach it). [`seal`] encrypts a 1:1 direct message so only the recipient
//! device can [`open`] it; the server routes opaque bytes it cannot read.
//!
//! Construction (boring on purpose — no invented primitives):
//! fresh ephemeral `X25519` → static-static `ECDH` → `HKDF-SHA256`
//! (`info = "terichat-dm-v1"`) → `XChaCha20-Poly1305`, then the sender signs
//! `ephemeral_pub || nonce || ciphertext` with its `Ed25519` identity key.
//!
//! Explicitly NOT provided here (see `docs/THREAT_MODEL.md`): forward secrecy
//! (recipient agreement key is long-term), deniability (messages are signed),
//! groups (needs `MLS`), key rotation/verification ceremonies, and replay
//! protection beyond the server's per-conversation sequence.

#![forbid(unsafe_code)]

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

/// `ephemeral_pub (32) + nonce (24) + signature (64)` envelope header.
pub const HEADER_LEN: usize = 32 + 24 + 64;

/// Server-side envelope cap is 1 MiB; plaintext must leave room for the header.
pub const MAX_PLAINTEXT_BYTES: usize = 1_048_576 - HEADER_LEN;

/// `HKDF` domain separation. Changing the construction retires this string.
const HKDF_INFO: &[u8] = b"terichat-dm-v1";

/// Typed device-crypto error. Messages carry no key or plaintext material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeriCryptError {
    /// System randomness unavailable.
    Randomness,
    /// Plaintext exceeds [`MAX_PLAINTEXT_BYTES`].
    TooLarge,
    /// Envelope shorter than [`HEADER_LEN`].
    InvalidEnvelope,
    /// Sender identity key does not parse.
    InvalidPublicKey,
    /// Signature check failed (forgery, wrong sender key, or tampering).
    InvalidSignature,
    /// Authenticated decryption failed (wrong recipient or tampering).
    DecryptFailed,
    /// Key-derivation internal failure (unreachable with fixed sizes).
    Internal,
}

impl std::fmt::Display for TeriCryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Randomness => write!(f, "system randomness unavailable"),
            Self::TooLarge => write!(f, "plaintext exceeds size limit"),
            Self::InvalidEnvelope => write!(f, "malformed envelope"),
            Self::InvalidPublicKey => write!(f, "invalid sender identity key"),
            Self::InvalidSignature => write!(f, "signature check failed"),
            Self::DecryptFailed => write!(f, "decryption failed"),
            Self::Internal => write!(f, "internal crypto failure"),
        }
    }
}

impl std::error::Error for TeriCryptError {}

/// Per-installation device identity. Never cloned across devices: each
/// installation generates its own.
pub struct IdentityKeypair {
    signing: SigningKey,
    agreement: StaticSecret,
}

impl IdentityKeypair {
    /// Generate a fresh identity from system randomness.
    ///
    /// # Errors
    ///
    /// Returns [`TeriCryptError::Randomness`] when the OS RNG is unavailable.
    pub fn generate() -> Result<Self, TeriCryptError> {
        let mut raw = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut raw)
            .map_err(|_| TeriCryptError::Randomness)?;
        Ok(Self {
            signing: SigningKey::from_bytes(&raw),
            agreement: StaticSecret::from(raw),
        })
    }

    /// `Ed25519` public identity key (what peers pin and verify against).
    #[must_use]
    pub fn identity_verify_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// `X25519` public agreement key (what peers seal to).
    #[must_use]
    pub fn agreement_pubkey(&self) -> [u8; 32] {
        PublicKey::from(&self.agreement).to_bytes()
    }

    /// Stable device fingerprint for future verification ceremonies
    /// (`hex(SHA-256(verify_key))`). Display only — never a secret.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        hex::encode(Sha256::digest(self.identity_verify_key()))
    }
}

/// Sealed 1:1 envelope. Opaque to everyone except the recipient device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedEnvelope {
    ephemeral_pub: [u8; 32],
    nonce: [u8; 24],
    signature: [u8; 64],
    ciphertext: Vec<u8>,
}

impl SealedEnvelope {
    /// Serialize: `ephemeral_pub || nonce || signature || ciphertext`.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.ciphertext.len());
        out.extend_from_slice(&self.ephemeral_pub);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.signature);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// Parse the wire layout. Rejects short inputs without panicking.
    ///
    /// # Errors
    ///
    /// Returns [`TeriCryptError::InvalidEnvelope`] below [`HEADER_LEN`] bytes.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, TeriCryptError> {
        if raw.len() < HEADER_LEN {
            return Err(TeriCryptError::InvalidEnvelope);
        }
        let mut ephemeral_pub = [0_u8; 32];
        let mut nonce = [0_u8; 24];
        let mut signature = [0_u8; 64];
        ephemeral_pub.copy_from_slice(&raw[..32]);
        nonce.copy_from_slice(&raw[32..56]);
        signature.copy_from_slice(&raw[56..120]);
        Ok(Self {
            ephemeral_pub,
            nonce,
            signature,
            ciphertext: raw[120..].to_vec(),
        })
    }
}

/// Derive the envelope key from an `ECDH` shared secret.
fn envelope_key(shared: &[u8], ephemeral_pub: &[u8; 32]) -> Result<[u8; 32], TeriCryptError> {
    let hk = Hkdf::<Sha256>::new(Some(ephemeral_pub), shared);
    let mut okm = [0_u8; 32];
    hk.expand(HKDF_INFO, &mut okm)
        .map_err(|_| TeriCryptError::Internal)?;
    Ok(okm)
}

fn random_bytes<const N: usize>() -> Result<[u8; N], TeriCryptError> {
    let mut buf = [0_u8; N];
    OsRng
        .try_fill_bytes(&mut buf)
        .map_err(|_| TeriCryptError::Randomness)?;
    Ok(buf)
}

/// Seal `plaintext` so only the holder of `recipient_agreement_pub` can open
/// it, signed by `sender`. Fresh ephemeral per call: identical plaintexts
/// produce unrelated envelopes.
///
/// # Errors
///
/// Returns [`TeriCryptError::TooLarge`] over [`MAX_PLAINTEXT_BYTES`], or
/// [`TeriCryptError::Randomness`] when the OS RNG is unavailable.
pub fn seal(
    sender: &IdentityKeypair,
    recipient_agreement_pub: &[u8; 32],
    plaintext: &[u8],
) -> Result<SealedEnvelope, TeriCryptError> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(TeriCryptError::TooLarge);
    }
    let ephemeral_priv: [u8; 32] = random_bytes()?;
    let ephemeral_pub = PublicKey::from(&StaticSecret::from(ephemeral_priv)).to_bytes();
    let shared = StaticSecret::from(ephemeral_priv)
        .diffie_hellman(&PublicKey::from(*recipient_agreement_pub));
    let key = envelope_key(shared.as_bytes(), &ephemeral_pub)?;
    let nonce: [u8; 24] = random_bytes()?;

    let aead = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| TeriCryptError::Internal)?;
    let ciphertext = aead
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|_| TeriCryptError::Internal)?;

    let mut signed = Vec::with_capacity(56 + ciphertext.len());
    signed.extend_from_slice(&ephemeral_pub);
    signed.extend_from_slice(&nonce);
    signed.extend_from_slice(&ciphertext);
    let signature = sender.signing.sign(&signed).to_bytes();

    Ok(SealedEnvelope {
        ephemeral_pub,
        nonce,
        signature,
        ciphertext,
    })
}

/// Open an envelope addressed to `recipient`, verifying it was signed by
/// `sender_verify_pub`. Every failure mode maps to a typed error — never a
/// panic, never a plaintext leak.
///
/// # Errors
///
/// Returns [`TeriCryptError::InvalidPublicKey`] for an unparsable sender key,
/// [`TeriCryptError::InvalidSignature`] on forgery or tampering, and
/// [`TeriCryptError::DecryptFailed`] when the recipient cannot open it.
pub fn open(
    recipient: &IdentityKeypair,
    sender_verify_pub: &[u8; 32],
    envelope: &SealedEnvelope,
) -> Result<Vec<u8>, TeriCryptError> {
    let sender = VerifyingKey::from_bytes(sender_verify_pub)
        .map_err(|_| TeriCryptError::InvalidPublicKey)?;
    let signature =
        Signature::from_slice(&envelope.signature).map_err(|_| TeriCryptError::InvalidSignature)?;

    let mut signed = Vec::with_capacity(56 + envelope.ciphertext.len());
    signed.extend_from_slice(&envelope.ephemeral_pub);
    signed.extend_from_slice(&envelope.nonce);
    signed.extend_from_slice(&envelope.ciphertext);
    sender
        .verify_strict(&signed, &signature)
        .map_err(|_| TeriCryptError::InvalidSignature)?;

    let shared = recipient
        .agreement
        .diffie_hellman(&PublicKey::from(envelope.ephemeral_pub));
    let key = envelope_key(shared.as_bytes(), &envelope.ephemeral_pub)?;
    XChaCha20Poly1305::new_from_slice(&key)
        .map_err(|_| TeriCryptError::Internal)?
        .decrypt(XNonce::from_slice(&envelope.nonce), &*envelope.ciphertext)
        .map_err(|_| TeriCryptError::DecryptFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> (IdentityKeypair, IdentityKeypair) {
        (
            IdentityKeypair::generate().expect("alice keys"),
            IdentityKeypair::generate().expect("bob keys"),
        )
    }

    #[test]
    fn roundtrip() {
        let (alice, bob) = devices();
        let envelope = seal(&alice, &bob.agreement_pubkey(), b"bro").expect("seal");
        let open_text = open(&bob, &alice.identity_verify_key(), &envelope).expect("open");
        assert_eq!(open_text, b"bro");
    }

    #[test]
    fn wire_format_roundtrips() {
        let (alice, bob) = devices();
        let envelope = seal(&alice, &bob.agreement_pubkey(), b"bro").expect("seal");
        let raw = envelope.to_bytes();
        assert!(raw.len() >= HEADER_LEN);
        let parsed = SealedEnvelope::from_bytes(&raw).expect("parse");
        assert_eq!(parsed, envelope);
        assert!(SealedEnvelope::from_bytes(&raw[..HEADER_LEN - 1]).is_err());
    }

    #[test]
    fn seals_are_randomized() {
        let (alice, bob) = devices();
        let first = seal(&alice, &bob.agreement_pubkey(), b"same").expect("seal");
        let second = seal(&alice, &bob.agreement_pubkey(), b"same").expect("seal");
        assert_ne!(first.to_bytes(), second.to_bytes());
    }

    #[test]
    fn wrong_recipient_cannot_open() {
        let (alice, bob) = devices();
        let mallory = IdentityKeypair::generate().expect("mallory keys");
        let envelope = seal(&alice, &bob.agreement_pubkey(), b"bro").expect("seal");
        let err = open(&mallory, &alice.identity_verify_key(), &envelope).unwrap_err();
        assert_eq!(err, TeriCryptError::DecryptFailed);
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let (alice, bob) = devices();
        let mut envelope = seal(&alice, &bob.agreement_pubkey(), b"bro").expect("seal");
        let last = envelope.ciphertext.len() - 1;
        envelope.ciphertext[last] ^= 0x01;
        // Signature covers the ciphertext, so this fails closed as forgery.
        let err = open(&bob, &alice.identity_verify_key(), &envelope).unwrap_err();
        assert_eq!(err, TeriCryptError::InvalidSignature);
    }

    #[test]
    fn tampered_signature_fails() {
        let (alice, bob) = devices();
        let mut envelope = seal(&alice, &bob.agreement_pubkey(), b"bro").expect("seal");
        envelope.signature[0] ^= 0x01;
        let err = open(&bob, &alice.identity_verify_key(), &envelope).unwrap_err();
        assert_eq!(err, TeriCryptError::InvalidSignature);
    }

    #[test]
    fn wrong_sender_key_fails() {
        let (alice, bob) = devices();
        let mallory = IdentityKeypair::generate().expect("mallory keys");
        let envelope = seal(&alice, &bob.agreement_pubkey(), b"bro").expect("seal");
        let err = open(&bob, &mallory.identity_verify_key(), &envelope).unwrap_err();
        assert_eq!(err, TeriCryptError::InvalidSignature);
    }

    #[test]
    fn fingerprints_are_stable_and_unique() {
        let (alice, bob) = devices();
        assert_eq!(alice.fingerprint(), alice.fingerprint());
        assert_eq!(alice.fingerprint().len(), 64);
        assert_ne!(alice.fingerprint(), bob.fingerprint());
    }
}
