//! `TeriCrypt-4096` device cryptography — Alpha proof of concept.
//!
//! Each installation owns an [`IdentityKeypair`]: an `Ed25519` signing key
//! (who the device claims to be) and an `X25519` agreement key (how others
//! reach it). [`seal`] encrypts a 1:1 direct message so only the recipient
//! device can [`open`] it; the server routes opaque bytes it cannot read.
//!
//! Construction (boring on purpose — no invented primitives):
//! fresh ephemeral `X25519` → static-static `ECDH` → `HKDF-SHA256`
//! (`salt = ephemeral_pub`, `info = "terichat-dm-v2" || sender_verify ||
//! recipient_agreement`) → `XChaCha20-Poly1305`, then the sender signs
//! `sender_verify || recipient_agreement || ephemeral_pub || nonce ||
//! ciphertext` with its `Ed25519` identity key. Identities are bound into
//! both the `KDF` and the signature, so a forwarded or re-signed envelope
//! never verifies as another conversation (`v1` bound neither; envelopes
//! are versioned by the `info` string and `v1` bytes do not open here).
//!
//! Key hygiene: the two long-term secrets are drawn independently (never one
//! seed for both groups), and all-zero peer keys are refused at seal time
//! and at device registration — a planted zero directory key cannot yield a
//! predictable envelope key. Non-zero low-order peer points are a documented
//! residual (see `docs/THREAT_MODEL.md`).
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
use zeroize::Zeroizing;

/// `ephemeral_pub (32) + nonce (24) + signature (64)` envelope header.
pub const HEADER_LEN: usize = 32 + 24 + 64;

/// Server-side envelope cap is 1 MiB; plaintext must leave room for the header.
pub const MAX_PLAINTEXT_BYTES: usize = 1_048_576 - HEADER_LEN;

/// `HKDF` domain separation. Changing the construction retires this string;
/// `v1` bound neither peer identity, so `v1` envelopes do not open here.
const HKDF_INFO: &[u8] = b"terichat-dm-v2";

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
    /// Agreement peer key is degenerate (all-zero): sealing under it would
    /// produce a predictable envelope key.
    InvalidAgreementKey,
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
            Self::InvalidAgreementKey => write!(f, "invalid agreement public key"),
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
    /// Generate a fresh identity from system randomness. The signing seed and
    /// the agreement secret are drawn independently: one 32-byte value must
    /// never seed both groups, or a break in either system's handling would
    /// expose the other identity.
    ///
    /// # Errors
    ///
    /// Returns [`TeriCryptError::Randomness`] when the OS RNG is unavailable.
    pub fn generate() -> Result<Self, TeriCryptError> {
        let sign_seed: Zeroizing<[u8; 32]> = Zeroizing::new(random_bytes()?);
        let agree_secret: Zeroizing<[u8; 32]> = Zeroizing::new(random_bytes()?);
        Ok(Self {
            signing: SigningKey::from_bytes(&sign_seed),
            agreement: StaticSecret::from(*agree_secret),
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

/// Whether `raw` is acceptable as an `Ed25519` device identity key:
/// canonically encoded (and non-zero — zero is never a real device key).
/// Non-zero low-order points parse here but fail closed at [`open`] via
/// `verify_strict`; full torsion rejection rides with the verification
/// ceremonies (see `docs/THREAT_MODEL.md`).
#[must_use]
pub fn valid_verify_key(raw: &[u8; 32]) -> bool {
    *raw != [0_u8; 32] && VerifyingKey::from_bytes(raw).is_ok()
}

/// Whether `raw` is acceptable as an `X25519` agreement key: anything but
/// the all-zero key, under which every envelope key would be predictable.
/// Non-zero low-order points are a documented residual (same reference).
#[must_use]
pub fn valid_agreement_key(raw: &[u8; 32]) -> bool {
    *raw != [0_u8; 32]
}

/// Derive the envelope key, binding both peer identities into the `HKDF`
/// context: the same bytes seal to different keys per direction, so a
/// forwarded envelope never opens as another conversation.
///
/// # Errors
///
/// Returns [`TeriCryptError::Internal`] when key expansion fails (unreachable
/// with fixed sizes).
fn envelope_key(
    shared: &[u8],
    ephemeral_pub: &[u8; 32],
    sender_verify: &[u8; 32],
    recipient_agreement: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, TeriCryptError> {
    let hk = Hkdf::<Sha256>::new(Some(ephemeral_pub), shared);
    let mut info = Vec::with_capacity(HKDF_INFO.len() + 64);
    info.extend_from_slice(HKDF_INFO);
    info.extend_from_slice(sender_verify);
    info.extend_from_slice(recipient_agreement);
    let mut okm = Zeroizing::new([0_u8; 32]);
    hk.expand(&info, &mut *okm)
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
/// produce unrelated envelopes. Sender and recipient identities are bound
/// into the key and the signature: re-signing these bytes speaks as the
/// forwarder, never as the original sender.
///
/// # Errors
///
/// Returns [`TeriCryptError::TooLarge`] over [`MAX_PLAINTEXT_BYTES`],
/// [`TeriCryptError::InvalidAgreementKey`] for a degenerate recipient key,
/// or [`TeriCryptError::Randomness`] when the OS RNG is unavailable.
pub fn seal(
    sender: &IdentityKeypair,
    recipient_agreement_pub: &[u8; 32],
    plaintext: &[u8],
) -> Result<SealedEnvelope, TeriCryptError> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(TeriCryptError::TooLarge);
    }
    // A zero recipient key (e.g. planted in the directory) would make the
    // envelope key predictable to whoever planted it. Refuse up front.
    if !valid_agreement_key(recipient_agreement_pub) {
        return Err(TeriCryptError::InvalidAgreementKey);
    }
    let ephemeral_priv: Zeroizing<[u8; 32]> = Zeroizing::new(random_bytes()?);
    let ephemeral = StaticSecret::from(*ephemeral_priv);
    let ephemeral_pub = PublicKey::from(&ephemeral).to_bytes();
    let shared = ephemeral.diffie_hellman(&PublicKey::from(*recipient_agreement_pub));
    let sender_verify = sender.identity_verify_key();
    let key = envelope_key(
        shared.as_bytes(),
        &ephemeral_pub,
        &sender_verify,
        recipient_agreement_pub,
    )?;
    let nonce: [u8; 24] = random_bytes()?;

    let aead = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| TeriCryptError::Internal)?;
    let ciphertext = aead
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|_| TeriCryptError::Internal)?;

    let mut signed = Vec::with_capacity(64 + 56 + ciphertext.len());
    signed.extend_from_slice(&sender_verify);
    signed.extend_from_slice(recipient_agreement_pub);
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
/// `sender_verify_pub` for this exact sender→recipient pair. Every failure
/// mode maps to a typed error — never a panic, never a plaintext leak.
///
/// # Errors
///
/// Returns [`TeriCryptError::InvalidPublicKey`] for an unparsable sender key,
/// [`TeriCryptError::InvalidSignature`] on forgery, re-signing, tampering,
/// or wrong sender key, and [`TeriCryptError::DecryptFailed`] when the
/// recipient cannot open it (wrong recipient, or an envelope sealed for a
/// different sender→recipient pair).
pub fn open(
    recipient: &IdentityKeypair,
    sender_verify_pub: &[u8; 32],
    envelope: &SealedEnvelope,
) -> Result<Vec<u8>, TeriCryptError> {
    let sender = VerifyingKey::from_bytes(sender_verify_pub)
        .map_err(|_| TeriCryptError::InvalidPublicKey)?;
    let signature =
        Signature::from_slice(&envelope.signature).map_err(|_| TeriCryptError::InvalidSignature)?;

    let recipient_agreement = PublicKey::from(&recipient.agreement).to_bytes();
    let mut signed = Vec::with_capacity(64 + 56 + envelope.ciphertext.len());
    signed.extend_from_slice(sender_verify_pub);
    signed.extend_from_slice(&recipient_agreement);
    signed.extend_from_slice(&envelope.ephemeral_pub);
    signed.extend_from_slice(&envelope.nonce);
    signed.extend_from_slice(&envelope.ciphertext);
    sender
        .verify_strict(&signed, &signature)
        .map_err(|_| TeriCryptError::InvalidSignature)?;

    let shared = recipient
        .agreement
        .diffie_hellman(&PublicKey::from(envelope.ephemeral_pub));
    let key = envelope_key(
        shared.as_bytes(),
        &envelope.ephemeral_pub,
        sender_verify_pub,
        &recipient_agreement,
    )?;
    XChaCha20Poly1305::new_from_slice(&*key)
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
        // The signature binds Bob's agreement key, so Mallory's verification
        // input differs and verification fails before decryption is reached.
        // Either gate failing closed is correct; this pins the earlier one.
        let err = open(&mallory, &alice.identity_verify_key(), &envelope).unwrap_err();
        assert_eq!(err, TeriCryptError::InvalidSignature);
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

    /// S1-2: a zero recipient key (e.g. planted in the directory) would make
    /// the envelope key predictable. Seal must refuse it, not encrypt under it.
    #[test]
    fn zero_recipient_key_is_refused() {
        let (alice, _) = devices();
        let err = seal(&alice, &[0_u8; 32], b"bro").unwrap_err();
        assert_eq!(err, TeriCryptError::InvalidAgreementKey);
    }

    /// S1-3: Mallory forwards Alice's bytes under her own signature. Nobody
    /// accepts them as Alice's — the signature binds the sender identity.
    #[test]
    fn resigned_forward_does_not_verify_as_sender() {
        let (alice, bob) = devices();
        let mallory = IdentityKeypair::generate().expect("mallory keys");
        let envelope = seal(&alice, &bob.agreement_pubkey(), b"bro").expect("seal");
        let forged = SealedEnvelope {
            ephemeral_pub: envelope.ephemeral_pub,
            nonce: envelope.nonce,
            signature: mallory
                .signing
                .sign(&signed_input(
                    &mallory.identity_verify_key(),
                    &bob.agreement_pubkey(),
                    &envelope,
                ))
                .to_bytes(),
            ciphertext: envelope.ciphertext.clone(),
        };
        let err = open(&bob, &alice.identity_verify_key(), &forged).unwrap_err();
        assert_eq!(err, TeriCryptError::InvalidSignature);
    }

    /// Registration gate: real keys pass, zero keys do not.
    #[test]
    fn key_validators_accept_real_reject_zero() {
        let device = IdentityKeypair::generate().expect("keys");
        assert!(valid_verify_key(&device.identity_verify_key()));
        assert!(valid_agreement_key(&device.agreement_pubkey()));
        assert!(!valid_verify_key(&[0_u8; 32]));
        assert!(!valid_agreement_key(&[0_u8; 32]));
    }

    /// The exact byte layout both sides sign and verify (v2 context first).
    /// Pinning it here means a layout change breaks loudly, not silently.
    fn signed_input(
        sender_verify: &[u8; 32],
        recipient_agreement: &[u8; 32],
        envelope: &SealedEnvelope,
    ) -> Vec<u8> {
        let mut signed = Vec::with_capacity(64 + 56 + envelope.ciphertext.len());
        signed.extend_from_slice(sender_verify);
        signed.extend_from_slice(recipient_agreement);
        signed.extend_from_slice(&envelope.ephemeral_pub);
        signed.extend_from_slice(&envelope.nonce);
        signed.extend_from_slice(&envelope.ciphertext);
        signed
    }
}
