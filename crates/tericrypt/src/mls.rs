//! Synthetic, client-local MLS 1.0 foundation using `OpenMLS` and `RustCrypto`.
//!
//! **Not production-ready.** Each installation owns independent signing keys
//! and the provider's non-durable `MemoryStorage`. No durable serialization/reload,
//! encrypted persistence, rollback protection, crash atomicity, or secure erasure
//! guarantee is provided. Dropping this state loses membership and messages.
//!
//! Credentials are fresh random scope-local labels, NOT authenticated account
//! or device identities. Callers in a synthetic harness supply trusted packages
//! and welcomes directly. There is no authentication service mapping, bootstrap,
//! approval ceremony, credential trust validation, or key transparency here.
//! Do not connect this API to an untrusted directory or a production transport.
//!
//! One installation is consumed into one group session, avoiding shared private
//! keys across installations or scopes. MLS wire bytes remain separate from the
//! existing sealed-DM API; failures never fall back to that protocol.

use openmls::prelude::{
    tls_codec::{Deserialize, Serialize},
    *,
};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;

/// The only suite this foundation creates or accepts; no negotiation/downgrade.
pub const SUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// Errors contain no upstream diagnostics, keys, or application content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlsError {
    /// Provider, randomness, or MLS operation failed.
    Operation,
    /// Invalid encoding or an unexpected MLS message kind.
    InvalidInput,
    /// The peer selected an unsupported suite.
    UnsupportedSuite,
}

impl std::fmt::Display for MlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Operation => "MLS operation failed",
            Self::InvalidInput => "invalid MLS input",
            Self::UnsupportedSuite => "unsupported MLS ciphersuite",
        })
    }
}
impl std::error::Error for MlsError {}

/// Independent synthetic installation; storage is volatile and not exported.
/// Neither this type nor its provider/key material is clonable or debug-printable.
pub struct MemoryInstallation {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
}

impl MemoryInstallation {
    /// Generate fresh installation keys and a random synthetic scoped identity.
    ///
    /// # Errors
    /// Returns `Operation` if key/identity generation fails.
    ///
    /// # Panics
    /// The upstream provider's default constructor may panic if OS entropy is unavailable.
    pub fn generate() -> Result<Self, MlsError> {
        let provider = OpenMlsRustCrypto::default();
        let signer =
            SignatureKeyPair::new(SUITE.signature_algorithm()).map_err(|_| MlsError::Operation)?;
        let identity = crate::random_bytes::<32>().map_err(|_| MlsError::Operation)?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(identity.to_vec()).into(),
            signature_key: signer.to_public_vec().into(),
        };
        Ok(Self {
            provider,
            signer,
            credential,
        })
    }

    /// Generate a signed, one-time MLS key package; private bundle stays in memory.
    ///
    /// # Errors
    /// Returns `Operation` on provider, storage, or encoding failure.
    pub fn key_package(&self) -> Result<Vec<u8>, MlsError> {
        KeyPackage::builder()
            .build(SUITE, &self.provider, &self.signer, self.credential.clone())
            .map_err(|_| MlsError::Operation)?
            .key_package()
            .tls_serialize_detached()
            .map_err(|_| MlsError::Operation)
    }

    /// Create a fresh group with an upstream-random group ID and the locked suite.
    ///
    /// # Errors
    /// Returns `Operation` on MLS creation failure.
    pub fn create_group(self) -> Result<Session, MlsError> {
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(SUITE)
            .use_ratchet_tree_extension(true)
            .build();
        let group = MlsGroup::new(
            &self.provider,
            &self.signer,
            &config,
            self.credential.clone(),
        )
        .map_err(|_| MlsError::Operation)?;
        Ok(Session {
            installation: self,
            group,
        })
    }

    /// Join a synthetic trusted Welcome using this installation's private package.
    /// Consumes the installation even on failure; retry/lifecycle design is pending.
    ///
    /// # Errors
    /// Rejects invalid messages, unsupported suites, or an MLS join failure.
    pub fn join(self, wire: &[u8]) -> Result<Session, MlsError> {
        let message =
            MlsMessageIn::tls_deserialize_exact(wire).map_err(|_| MlsError::InvalidInput)?;
        let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
            return Err(MlsError::InvalidInput);
        };
        if welcome.ciphersuite() != SUITE {
            return Err(MlsError::UnsupportedSuite);
        }
        let config = MlsGroupJoinConfig::builder()
            .use_ratchet_tree_extension(true)
            .build();
        let group = StagedWelcome::new_from_welcome(&self.provider, &config, welcome, None)
            .map_err(|_| MlsError::Operation)?
            .into_group(&self.provider)
            .map_err(|_| MlsError::Operation)?;
        Ok(Session {
            installation: self,
            group,
        })
    }
}

/// One installation's in-memory group state, never shared with the server.
pub struct Session {
    installation: MemoryInstallation,
    group: MlsGroup,
}

/// Serialized MLS artifacts for ordered delivery by a synthetic harness.
/// The creator has already merged locally; deliver commit to existing peers
/// before sending new-epoch application messages, and Welcome to new peers.
pub struct Addition {
    /// Commit for existing peers (not the creator).
    pub commit: Vec<u8>,
    /// Welcome for newly added installations only.
    pub welcome: Vec<u8>,
}

/// Successfully processed MLS content. Plaintext must remain client-local.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// Authenticated application plaintext (not an authenticated account claim).
    Application(Vec<u8>),
    /// A valid membership commit was merged, possibly removing this installation.
    EpochChanged,
}

impl Session {
    /// Remove the member with this exact synthetic scoped label and merge locally.
    /// The caller must deliver the returned commit to peers in order. This is not
    /// an account-level authorization policy or remote wipe operation.
    ///
    /// # Errors
    /// Rejects absent/ambiguous labels or invalid MLS operations.
    pub fn remove(&mut self, identity: &[u8]) -> Result<Vec<u8>, MlsError> {
        let member = {
            let mut matches = self
                .group
                .members()
                .filter(|member| member.credential.serialized_content() == identity);
            let member = matches.next().ok_or(MlsError::InvalidInput)?;
            if matches.next().is_some() {
                return Err(MlsError::InvalidInput);
            }
            member
        };
        let provider = &self.installation.provider;
        let (commit, _, _) = self
            .group
            .remove_members(provider, &self.installation.signer, &[member.index])
            .map_err(|_| MlsError::Operation)?;
        let wire = commit.to_bytes().map_err(|_| MlsError::Operation)?;
        self.group
            .merge_pending_commit(provider)
            .map_err(|_| MlsError::Operation)?;
        Ok(wire)
    }

    /// Add packages supplied by the trusted synthetic harness and merge locally.
    /// MLS signature validation does NOT establish account/device authorization.
    ///
    /// # Errors
    /// Rejects malformed packages, unsupported suites, and invalid MLS operations.
    pub fn add(&mut self, packages: &[Vec<u8>]) -> Result<Addition, MlsError> {
        let provider = &self.installation.provider;
        let packages = packages
            .iter()
            .map(|wire| {
                let package = KeyPackageIn::tls_deserialize_exact(wire)
                    .map_err(|_| MlsError::InvalidInput)?
                    .validate(provider.crypto(), ProtocolVersion::Mls10)
                    .map_err(|_| MlsError::InvalidInput)?;
                if package.ciphersuite() != SUITE {
                    return Err(MlsError::UnsupportedSuite);
                }
                Ok(package)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (commit, welcome, _) = self
            .group
            .add_members(provider, &self.installation.signer, &packages)
            .map_err(|_| MlsError::Operation)?;
        let addition = Addition {
            commit: commit.to_bytes().map_err(|_| MlsError::Operation)?,
            welcome: welcome.to_bytes().map_err(|_| MlsError::Operation)?,
        };
        self.group
            .merge_pending_commit(provider)
            .map_err(|_| MlsError::Operation)?;
        Ok(addition)
    }

    /// Rotate this installation's leaf keys (MLS self-update) and merge locally.
    /// The caller must deliver the returned commit to peers in order. Peers
    /// keep the same membership; only the epoch and leaf keys advance. This
    /// is key hygiene, not a credential/account change or a revocation.
    ///
    /// # Errors
    /// Returns `Operation` on MLS update, encoding, or merge failure.
    pub fn rotate(&mut self) -> Result<Vec<u8>, MlsError> {
        let provider = &self.installation.provider;
        let params = LeafNodeParameters::builder().build();
        let bundle = self
            .group
            .self_update(provider, &self.installation.signer, params)
            .map_err(|_| MlsError::Operation)?;
        let wire = bundle
            .commit()
            .to_bytes()
            .map_err(|_| MlsError::Operation)?;
        self.group
            .merge_pending_commit(provider)
            .map_err(|_| MlsError::Operation)?;
        Ok(wire)
    }

    /// Encrypt application plaintext as an MLS message, never as a `PoC` envelope.
    ///
    /// # Errors
    /// Returns `InvalidInput` above 64 KiB or `Operation` on encryption/encoding failure.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, MlsError> {
        // Local experiment limit, not the existing server envelope contract.
        if plaintext.len() > 65_536 {
            return Err(MlsError::InvalidInput);
        }
        self.group
            .create_message(
                &self.installation.provider,
                &self.installation.signer,
                plaintext,
            )
            .map_err(|_| MlsError::Operation)?
            .to_bytes()
            .map_err(|_| MlsError::Operation)
    }

    /// Process an MLS application message or merge a valid commit from a group peer.
    /// No trust-directory integration or application-level membership policy exists.
    ///
    /// # Errors
    /// Rejects malformed, foreign-group, unauthenticated, or unexpected messages.
    pub fn receive(&mut self, wire: &[u8]) -> Result<Received, MlsError> {
        let message = MlsMessageIn::tls_deserialize_exact(wire)
            .map_err(|_| MlsError::InvalidInput)?
            .try_into_protocol_message()
            .map_err(|_| MlsError::InvalidInput)?;
        let Ok(processed) = self
            .group
            .process_message(&self.installation.provider, message)
        else {
            // OpenMLS can mutate its live receive ratchet before AEAD failure,
            // without updating storage. Reconcile with the provider's latest
            // state, never an earlier snapshot. Successful stored generations
            // remain consumed; this is not a transaction or durable rollback.
            self.group =
                MlsGroup::load(self.installation.provider.storage(), self.group.group_id())
                    .map_err(|_| MlsError::Operation)?
                    .ok_or(MlsError::Operation)?;
            return Err(MlsError::Operation);
        };
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(message) => {
                Ok(Received::Application(message.into_bytes()))
            }
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                self.group
                    .merge_staged_commit(&self.installation.provider, *commit)
                    .map_err(|_| MlsError::Operation)?;
                Ok(Received::EpochChanged)
            }
            _ => Err(MlsError::InvalidInput),
        }
    }

    /// Current epoch (public metadata).
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.group.epoch().as_u64()
    }

    /// Current member count (each installation is a member).
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.group.members().count()
    }

    /// Suite in the actual MLS group context.
    #[must_use]
    pub fn ciphersuite(&self) -> Ciphersuite {
        self.group.ciphersuite()
    }

    /// Public signing key, for synthetic independence assertions only.
    #[must_use]
    pub fn signature_public_key(&self) -> Vec<u8> {
        self.installation.signer.to_public_vec()
    }

    /// Random scope-local credential label, not an authenticated account identity.
    #[must_use]
    pub fn scoped_identity(&self) -> Vec<u8> {
        self.installation
            .credential
            .credential
            .serialized_content()
            .to_vec()
    }
}
