//! Encrypted SQLite persistence for the synthetic MLS experiment (issue #6 S1).
//!
//! **Not production-ready.** This keeps the [`crate::mls`] trust model
//! (fresh random scope-local labels, no account binding, no audits) and only
//! swaps where bytes rest: the volatile `MemoryStorage` inside
//! `OpenMlsRustCrypto` is replaced by [`EncryptedStore`], an OpenMLS
//! [`StorageProvider`](openmls_traits::storage::StorageProvider) backed by a
//! file SQLite database whose values are encrypted at rest.
//!
//! ## Volatility contrast
//!
//! `MemoryStorage` (see [`crate::mls`] docs) holds group state in a process
//! `HashMap`: dropping the provider loses membership and message secrets, so
//! a restarted client cannot continue any group. [`PersistSession`] instead
//! records every OpenMLS write plus the installation's signer, scoped
//! identity, and group id into the encrypted database; dropping the whole
//! provider and reopening the same file with the same login password restores
//! the session at the same epoch and delivery continues in both directions.
//!
//! ## Algorithms and parameters (locked, boring on purpose)
//!
//! * Key derivation: Argon2id via `argon2::Argon2::default()` — the same
//!   baseline the server login path uses (Argon2id v1.3, 19 MiB memory,
//!   2 iterations, 1 lane, per the `argon2` 0.5 defaults) — stretched to a
//!   32-byte database key. The caller passes the login password bytes; the
//!   password itself is never stored, only a fresh random salt per file.
//! * At-rest encryption: XChaCha20-Poly1305 (`chacha20poly1305` 0.10, already
//!   a dependency — no new crypto). Every stored value gets a fresh random
//!   24-byte nonce; the blob on disk is `nonce || ciphertext`, so equal
//!   plaintexts never look alike and any flipped byte fails authentication.
//!   The storage row key rides as AEAD associated data, so swapping two
//!   valid blobs between rows fails closed instead of decrypting wrong.
//! * Row keys and metadata labels stay plaintext on disk (group IDs, epochs,
//!   row counts, sizes are visible to a file thief); only values are sealed.
//!   Read "encrypted at rest" as values-only unless the threat model grows.
//! * Key layout mirrors `MemoryStorage` (`label || serde_json(key) ||
//!   big-endian version`) with JSON-serialized entities; list-shaped entries
//!   (own leaf nodes, proposal queues, epoch key pairs) keep the same
//!   read-modify-write list semantics, executed here inside a SQLite
//!   transaction on a single-connection pool.
//!
//! ## Limits (explicit non-goals for S1)
//!
//! * Sync only: OpenMLS calls storage synchronously, so an internal Tokio
//!   runtime drives `sqlx`. Calling from inside an async runtime fails closed
//!   ([`StoreError::SyncContext`]) instead of deadlocking.
//! * One active group per database file (the last created/joined group id is
//!   remembered for [`PersistSession::reopen`]); multi-group files, schema
//!   migrations, rollback protection, crash atomicity beyond SQLite's own
//!   guarantees, and secure erasure are all future work.
//! * Test passwords are synthetic fixtures. Real login integration, account
//!   binding, audits, and any production claim are out of scope.

use std::future::Future;
use std::path::{Path, PathBuf};

use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use openmls::prelude::{tls_codec::Deserialize, tls_codec::Serialize, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::{
    storage::{traits as store_traits, Entity, Key, StorageProvider, CURRENT_VERSION},
    OpenMlsProvider,
};
use rand_core::{OsRng, RngCore};
use sqlx::{sqlite::SqliteConnectOptions, sqlite::SqlitePoolOptions, Row, SqlitePool};
use zeroize::Zeroizing;

use crate::mls::{MlsError, SUITE};

/// Random salt bytes per database file (non-secret, stored alongside).
const SALT_LEN: usize = 16;
/// XChaCha20-Poly1305 nonce bytes, fresh per stored value.
const NONCE_LEN: usize = 24;
/// Expected shape of the password-check sentinel after decryption.
const VERIFY_PLAINTEXT: &[u8] = b"terichat-mls-persist-v1-ok";

/// Typed persistence error. Messages carry row counts at most: never keys,
/// passwords, nonces, plaintext, or ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// `SQLite` connection, schema, or statement failure.
    Database,
    /// Key derivation, randomness, or AEAD failure — including wrong password
    /// and tampered bytes. Both fail closed with no further detail.
    Sealed,
    /// JSON encoding/decoding failure for keys or entities.
    Serialization,
    /// Called from inside an async runtime, which the internal blocking
    /// executor cannot serve. Use a synchronous context.
    SyncContext,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Database => "persistent store unavailable",
            Self::Sealed => "persistent store authentication failed",
            Self::Serialization => "persistent store entry malformed",
            Self::SyncContext => "persistent store requires a synchronous context",
        })
    }
}

impl std::error::Error for StoreError {}

// Storage labels mirror `MemoryStorage` so semantics stay comparable.
const KEY_PACKAGE_LABEL: &[u8] = b"KeyPackage";
const PSK_LABEL: &[u8] = b"Psk";
const ENCRYPTION_KEY_PAIR_LABEL: &[u8] = b"EncryptionKeyPair";
const SIGNATURE_KEY_PAIR_LABEL: &[u8] = b"SignatureKeyPair";
const EPOCH_KEY_PAIRS_LABEL: &[u8] = b"EpochKeyPairs";
const TREE_LABEL: &[u8] = b"Tree";
const GROUP_CONTEXT_LABEL: &[u8] = b"GroupContext";
const INTERIM_TRANSCRIPT_HASH_LABEL: &[u8] = b"InterimTranscriptHash";
const CONFIRMATION_TAG_LABEL: &[u8] = b"ConfirmationTag";
const JOIN_CONFIG_LABEL: &[u8] = b"MlsGroupJoinConfig";
const OWN_LEAF_NODES_LABEL: &[u8] = b"OwnLeafNodes";
const GROUP_STATE_LABEL: &[u8] = b"GroupState";
const QUEUED_PROPOSAL_LABEL: &[u8] = b"QueuedProposal";
const PROPOSAL_QUEUE_REFS_LABEL: &[u8] = b"ProposalQueueRefs";
const OWN_LEAF_NODE_INDEX_LABEL: &[u8] = b"OwnLeafNodeIndex";
const EPOCH_SECRETS_LABEL: &[u8] = b"EpochSecrets";
const RESUMPTION_PSK_STORE_LABEL: &[u8] = b"ResumptionPsk";
const MESSAGE_SECRETS_LABEL: &[u8] = b"MessageSecrets";

// `mls_meta` rows. The salt is non-secret by design; every other row below
// (including the signer's private key material) is AEAD-sealed.
const META_SALT: &str = "salt";
const META_VERIFY: &str = "verify";
const META_SIGNER: &str = "app/signer";
const META_IDENTITY: &str = "app/identity";
const META_SIG_PUB: &str = "app/sigpub";
const META_GROUP: &str = "app/group";

/// `SQLite`-backed `OpenMLS` storage with every value sealed at rest.
///
/// The database key is derived per open from the login password and the
/// file's salt, held only in memory as [`Zeroizing`], and never written to
/// disk. Neither this type nor anything holding it is clonable, and its
/// `Debug` output redacts key material.
pub struct EncryptedStore {
    pool: SqlitePool,
    runtime: tokio::runtime::Runtime,
    key: Zeroizing<[u8; 32]>,
    salt: [u8; SALT_LEN],
    #[allow(dead_code)]
    path: PathBuf,
}

impl std::fmt::Debug for EncryptedStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedStore")
            .field("key", &"***")
            .field("salt", &"***")
            .finish_non_exhaustive()
    }
}

/// Derive the 32-byte database key with Argon2id (`Argon2::default()`).
fn derive_key(password: &[u8], salt: &[u8; SALT_LEN]) -> Result<Zeroizing<[u8; 32]>, StoreError> {
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(password, salt, &mut key[..])
        .map_err(|_| StoreError::Sealed)?;
    Ok(key)
}

fn storage_key(label: &[u8], key_json: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(label.len() + key_json.len() + 2);
    out.extend_from_slice(label);
    out.extend_from_slice(key_json);
    out.extend_from_slice(&u16::to_be_bytes(CURRENT_VERSION));
    out
}

fn key_json<K: Key<CURRENT_VERSION>>(key: &K) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(key).map_err(|_| StoreError::Serialization)
}

impl EncryptedStore {
    /// Open (or create) the encrypted store at `path`, deriving its key from
    /// `password` (the login password bytes; synthetic fixtures in tests).
    ///
    /// A fresh file records a new random salt and a password-check sentinel.
    /// An existing file reuses its salt and rejects a wrong password (or
    /// tampered sentinel bytes) with [`StoreError::Sealed`]: fail closed, no
    /// group state is exposed.
    ///
    /// # Errors
    ///
    /// Returns `Database` when the file cannot be opened or the schema cannot
    /// be prepared, `Sealed` on wrong password/tamper/RNG failure, and
    /// `Serialization` when app-state bytes do not encode.
    pub fn open(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, StoreError> {
        let runtime = tokio::runtime::Runtime::new().map_err(|_| StoreError::Database)?;
        let path = path.as_ref().to_path_buf();
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = runtime
            .block_on(
                SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect_with(options),
            )
            .map_err(|_| StoreError::Database)?;
        let store = Self {
            pool,
            runtime,
            key: Zeroizing::new([0u8; 32]),
            salt: [0u8; SALT_LEN],
            path,
        };
        store.block(async {
            for ddl in [
                "CREATE TABLE IF NOT EXISTS mls_kv (key BLOB PRIMARY KEY, val BLOB NOT NULL)",
                "CREATE TABLE IF NOT EXISTS mls_meta (key TEXT PRIMARY KEY, val BLOB NOT NULL)",
            ] {
                sqlx::query(ddl)
                    .execute(&store.pool)
                    .await
                    .map_err(|_| StoreError::Database)?;
            }
            Ok::<(), StoreError>(())
        })?;
        // Load the file salt, or mint a fresh one exactly once per file.
        // Reopening never regenerates it: salt reuse across files is rejected
        // by construction (fresh `OsRng` bytes per new file).
        let salt: [u8; SALT_LEN] = if let Some(raw) = store.meta_get(META_SALT)? {
            raw.try_into().map_err(|_| StoreError::Serialization)?
        } else {
            let mut fresh = [0u8; SALT_LEN];
            OsRng
                .try_fill_bytes(&mut fresh)
                .map_err(|_| StoreError::Sealed)?;
            store.meta_put(META_SALT, &fresh)?;
            fresh
        };
        let key = derive_key(password, &salt)?;
        let store = Self { key, salt, ..store };
        // Password check: fresh files seal the sentinel, existing files must
        // open it. Wrong passwords and tampered bytes both fail here.
        match store.meta_get(META_VERIFY)? {
            None => {
                let sealed = store.seal(META_VERIFY.as_bytes(), VERIFY_PLAINTEXT)?;
                store.meta_put(META_VERIFY, &sealed)?;
            }
            Some(sealed) => {
                let plain = store.unseal(META_VERIFY.as_bytes(), &sealed)?;
                if plain.as_slice() != VERIFY_PLAINTEXT {
                    return Err(StoreError::Sealed);
                }
            }
        }
        Ok(store)
    }

    /// The file's non-secret salt (exposed so tests can assert fresh salt per
    /// database; salts are not secret and reveal nothing without the password).
    #[must_use]
    pub fn salt(&self) -> [u8; SALT_LEN] {
        self.salt
    }

    /// Drive an async `sqlx` future from the synchronous storage-trait methods.
    /// Fails closed when called inside an async runtime instead of blocking it.
    fn block<T>(
        &self,
        future: impl Future<Output = Result<T, StoreError>>,
    ) -> Result<T, StoreError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(StoreError::SyncContext);
        }
        self.runtime.block_on(future)
    }

    /// Seal plaintext as `nonce || ciphertext` under the database key, bound
    /// to the row it will live under: the storage key rides as AEAD
    /// associated data, so swapping two valid blobs between rows fails
    /// authentication instead of decrypting into the wrong slot.
    fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, StoreError> {
        let mut nonce = [0u8; NONCE_LEN];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| StoreError::Sealed)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(&self.key[..]).map_err(|_| StoreError::Sealed)?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| StoreError::Sealed)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Open a sealed blob bound to `aad`; truncation, flips, row swaps, and
    /// wrong keys all fail closed.
    fn unseal(&self, aad: &[u8], blob: &[u8]) -> Result<Vec<u8>, StoreError> {
        let Some((nonce, ciphertext)) = blob.split_at_checked(NONCE_LEN) else {
            return Err(StoreError::Sealed);
        };
        let cipher =
            XChaCha20Poly1305::new_from_slice(&self.key[..]).map_err(|_| StoreError::Sealed)?;
        cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| StoreError::Sealed)
    }

    fn kv_put(&self, key: &[u8], plaintext: &[u8]) -> Result<(), StoreError> {
        let sealed = self.seal(key, plaintext)?;
        self.block(async {
            sqlx::query(
                "INSERT INTO mls_kv(key, val) VALUES (?, ?) \
                 ON CONFLICT(key) DO UPDATE SET val = excluded.val",
            )
            .bind(key)
            .bind(sealed.as_slice())
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|_| StoreError::Database)
        })
    }

    fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let sealed: Option<Vec<u8>> = self.block(async {
            sqlx::query("SELECT val FROM mls_kv WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| StoreError::Database)
                .map(|row| row.map(|row| row.get::<Vec<u8>, usize>(0)))
        })?;
        sealed.map(|blob| self.unseal(key, &blob)).transpose()
    }

    fn kv_del(&self, key: &[u8]) -> Result<(), StoreError> {
        self.block(async {
            sqlx::query("DELETE FROM mls_kv WHERE key = ?")
                .bind(key)
                .execute(&self.pool)
                .await
                .map(|_| ())
                .map_err(|_| StoreError::Database)
        })
    }

    fn meta_get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.block(async {
            sqlx::query("SELECT val FROM mls_meta WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| StoreError::Database)
                .map(|row| row.map(|row| row.get::<Vec<u8>, usize>(0)))
        })
    }

    fn meta_put(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        self.block(async {
            sqlx::query(
                "INSERT INTO mls_meta(key, val) VALUES (?, ?) \
                 ON CONFLICT(key) DO UPDATE SET val = excluded.val",
            )
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|_| StoreError::Database)
        })
    }

    fn meta_put_sealed(&self, key: &str, plaintext: &[u8]) -> Result<(), StoreError> {
        let sealed = self.seal(key.as_bytes(), plaintext)?;
        self.meta_put(key, &sealed)
    }

    fn meta_get_sealed(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.meta_get(key)?
            .map(|blob| self.unseal(key.as_bytes(), &blob))
            .transpose()
    }

    /// Store already-serialized JSON bytes (for values that are not a single
    /// `Entity`: proposal blobs, epoch key-pair lists, JSON lists).
    fn raw_put(&self, skey: &[u8], plain_json: &[u8]) -> Result<(), StoreError> {
        self.kv_put(skey, plain_json)
    }

    /// Load already-serialized JSON bytes; `None` when the row is absent.
    fn raw_get(&self, skey: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.kv_get(skey)
    }

    fn entity_put<V: Entity<CURRENT_VERSION>>(
        &self,
        label: &[u8],
        key: &[u8],
        value: &V,
    ) -> Result<(), StoreError> {
        let plain = serde_json::to_vec(value).map_err(|_| StoreError::Serialization)?;
        self.kv_put(&storage_key(label, key), &plain)
    }

    fn entity_get<V: Entity<CURRENT_VERSION>>(
        &self,
        label: &[u8],
        key: &[u8],
    ) -> Result<Option<V>, StoreError> {
        self.kv_get(&storage_key(label, key))?
            .map(|plain| serde_json::from_slice(&plain).map_err(|_| StoreError::Serialization))
            .transpose()
    }

    /// Read-modify-write a JSON list entry inside one transaction so
    /// concurrent `&self` writers cannot interleave.
    fn list_update(
        &self,
        label: &[u8],
        key: &[u8],
        item: &[u8],
        append: bool,
    ) -> Result<(), StoreError> {
        let storage_key = storage_key(label, key);
        let key = self.key.clone();
        self.block(async {
            let mut tx = self.pool.begin().await.map_err(|_| StoreError::Database)?;
            let current: Option<Vec<u8>> = sqlx::query("SELECT val FROM mls_kv WHERE key = ?")
                .bind(storage_key.as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| StoreError::Database)
                .map(|row| row.map(|row| row.get::<Vec<u8>, usize>(0)))?;
            // Decrypt inline: the transaction future cannot borrow `self`.
            let mut list: Vec<Vec<u8>> = match current {
                None => Vec::new(),
                Some(blob) => {
                    let Some((nonce, ciphertext)) = blob.split_at_checked(NONCE_LEN) else {
                        return Err(StoreError::Sealed);
                    };
                    let cipher = XChaCha20Poly1305::new_from_slice(&key[..])
                        .map_err(|_| StoreError::Sealed)?;
                    let plain = cipher
                        .decrypt(XNonce::from_slice(nonce), ciphertext)
                        .map_err(|_| StoreError::Sealed)?;
                    serde_json::from_slice(&plain).map_err(|_| StoreError::Serialization)?
                }
            };
            if append {
                list.push(item.to_vec());
            } else if let Some(pos) = list.iter().position(|stored| stored.as_slice() == item) {
                list.remove(pos);
            }
            let plain = serde_json::to_vec(&list).map_err(|_| StoreError::Serialization)?;
            let mut nonce = [0u8; NONCE_LEN];
            OsRng
                .try_fill_bytes(&mut nonce)
                .map_err(|_| StoreError::Sealed)?;
            let cipher =
                XChaCha20Poly1305::new_from_slice(&key[..]).map_err(|_| StoreError::Sealed)?;
            let ciphertext = cipher
                .encrypt(XNonce::from_slice(&nonce), plain.as_slice())
                .map_err(|_| StoreError::Sealed)?;
            let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
            sealed.extend_from_slice(&nonce);
            sealed.extend_from_slice(&ciphertext);
            sqlx::query(
                "INSERT INTO mls_kv(key, val) VALUES (?, ?) \
                 ON CONFLICT(key) DO UPDATE SET val = excluded.val",
            )
            .bind(storage_key.as_slice())
            .bind(sealed.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Database)?;
            tx.commit().await.map_err(|_| StoreError::Database)?;
            Ok(())
        })
    }

    fn list_get<V: Entity<CURRENT_VERSION>>(
        &self,
        label: &[u8],
        key: &[u8],
    ) -> Result<Vec<V>, StoreError> {
        let items: Vec<Vec<u8>> = self
            .raw_get(&storage_key(label, key))?
            .map(|plain| serde_json::from_slice(&plain).map_err(|_| StoreError::Serialization))
            .transpose()?
            .unwrap_or_default();
        items
            .iter()
            .map(|bytes| serde_json::from_slice(bytes).map_err(|_| StoreError::Serialization))
            .collect()
    }

    /// Persist the installation identity (signer, scoped label, group id)
    /// alongside the MLS state so [`PersistSession::reopen`] can restore it.
    /// Everything here is sealed; the salt row stays the only raw meta row.
    fn save_app_state(
        &self,
        signer: &SignatureKeyPair,
        identity: &[u8],
        signature_public: &[u8],
        group_id: &[u8],
    ) -> Result<(), StoreError> {
        let signer_json = serde_json::to_vec(signer).map_err(|_| StoreError::Serialization)?;
        self.meta_put_sealed(META_SIGNER, &signer_json)?;
        self.meta_put_sealed(META_IDENTITY, identity)?;
        self.meta_put_sealed(META_SIG_PUB, signature_public)?;
        self.meta_put_sealed(META_GROUP, group_id)?;
        Ok(())
    }

    fn load_app_state(&self) -> Result<(SignatureKeyPair, CredentialWithKey, Vec<u8>), StoreError> {
        let signer_json = self
            .meta_get_sealed(META_SIGNER)?
            .ok_or(StoreError::Sealed)?;
        let signer: SignatureKeyPair =
            serde_json::from_slice(&signer_json).map_err(|_| StoreError::Serialization)?;
        let identity = self
            .meta_get_sealed(META_IDENTITY)?
            .ok_or(StoreError::Sealed)?;
        let signature_public = self
            .meta_get_sealed(META_SIG_PUB)?
            .ok_or(StoreError::Sealed)?;
        let group_id = self
            .meta_get_sealed(META_GROUP)?
            .ok_or(StoreError::Sealed)?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(identity).into(),
            signature_key: signature_public.into(),
        };
        Ok((signer, credential, group_id))
    }
}

fn epoch_pairs_id(group_id_json: &[u8], epoch_json: &[u8], leaf_index_json: &[u8]) -> Vec<u8> {
    let mut key = group_id_json.to_vec();
    key.extend_from_slice(epoch_json);
    key.extend_from_slice(leaf_index_json);
    key
}

impl StorageProvider<CURRENT_VERSION> for EncryptedStore {
    type Error = StoreError;

    fn write_mls_join_config<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        MlsGroupJoinConfig: store_traits::MlsGroupJoinConfig<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        config: &MlsGroupJoinConfig,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(JOIN_CONFIG_LABEL, &key, config)
    }

    fn append_own_leaf_node<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        LeafNode: store_traits::LeafNode<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        leaf_node: &LeafNode,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        let item = serde_json::to_vec(leaf_node).map_err(|_| StoreError::Serialization)?;
        self.list_update(OWN_LEAF_NODES_LABEL, &key, &item, true)
    }

    fn queue_proposal<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ProposalRef: store_traits::ProposalRef<CURRENT_VERSION>,
        QueuedProposal: store_traits::QueuedProposal<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        proposal_ref: &ProposalRef,
        proposal: &QueuedProposal,
    ) -> Result<(), Self::Error> {
        let key =
            serde_json::to_vec(&(group_id, proposal_ref)).map_err(|_| StoreError::Serialization)?;
        let value = serde_json::to_vec(proposal).map_err(|_| StoreError::Serialization)?;
        self.raw_put(&storage_key(QUEUED_PROPOSAL_LABEL, &key), &value)?;
        let key = key_json(group_id)?;
        let item = serde_json::to_vec(proposal_ref).map_err(|_| StoreError::Serialization)?;
        self.list_update(PROPOSAL_QUEUE_REFS_LABEL, &key, &item, true)
    }

    fn write_tree<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        TreeSync: store_traits::TreeSync<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        tree: &TreeSync,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(TREE_LABEL, &key, tree)
    }

    fn write_interim_transcript_hash<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        InterimTranscriptHash: store_traits::InterimTranscriptHash<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        interim_transcript_hash: &InterimTranscriptHash,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(INTERIM_TRANSCRIPT_HASH_LABEL, &key, interim_transcript_hash)
    }

    fn write_context<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        GroupContext: store_traits::GroupContext<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        group_context: &GroupContext,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(GROUP_CONTEXT_LABEL, &key, group_context)
    }

    fn write_confirmation_tag<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ConfirmationTag: store_traits::ConfirmationTag<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        confirmation_tag: &ConfirmationTag,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(CONFIRMATION_TAG_LABEL, &key, confirmation_tag)
    }

    fn write_group_state<
        GroupState: store_traits::GroupState<CURRENT_VERSION>,
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        group_state: &GroupState,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(GROUP_STATE_LABEL, &key, group_state)
    }

    fn write_message_secrets<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        MessageSecrets: store_traits::MessageSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        message_secrets: &MessageSecrets,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(MESSAGE_SECRETS_LABEL, &key, message_secrets)
    }

    fn write_resumption_psk_store<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ResumptionPskStore: store_traits::ResumptionPskStore<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        resumption_psk_store: &ResumptionPskStore,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(RESUMPTION_PSK_STORE_LABEL, &key, resumption_psk_store)
    }

    fn write_own_leaf_index<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        LeafNodeIndex: store_traits::LeafNodeIndex<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        own_leaf_index: &LeafNodeIndex,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(OWN_LEAF_NODE_INDEX_LABEL, &key, own_leaf_index)
    }

    fn write_group_epoch_secrets<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        GroupEpochSecrets: store_traits::GroupEpochSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        group_epoch_secrets: &GroupEpochSecrets,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.entity_put(EPOCH_SECRETS_LABEL, &key, group_epoch_secrets)
    }

    fn write_signature_key_pair<
        SignaturePublicKey: store_traits::SignaturePublicKey<CURRENT_VERSION>,
        SignatureKeyPair: store_traits::SignatureKeyPair<CURRENT_VERSION>,
    >(
        &self,
        public_key: &SignaturePublicKey,
        signature_key_pair: &SignatureKeyPair,
    ) -> Result<(), Self::Error> {
        let key = key_json(public_key)?;
        self.entity_put(SIGNATURE_KEY_PAIR_LABEL, &key, signature_key_pair)
    }

    fn write_encryption_key_pair<
        EncryptionKey: store_traits::EncryptionKey<CURRENT_VERSION>,
        HpkeKeyPair: store_traits::HpkeKeyPair<CURRENT_VERSION>,
    >(
        &self,
        public_key: &EncryptionKey,
        key_pair: &HpkeKeyPair,
    ) -> Result<(), Self::Error> {
        let key = key_json(public_key)?;
        self.entity_put(ENCRYPTION_KEY_PAIR_LABEL, &key, key_pair)
    }

    fn write_encryption_epoch_key_pairs<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        EpochKey: store_traits::EpochKey<CURRENT_VERSION>,
        HpkeKeyPair: store_traits::HpkeKeyPair<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        epoch: &EpochKey,
        leaf_index: u32,
        key_pairs: &[HpkeKeyPair],
    ) -> Result<(), Self::Error> {
        let key = epoch_pairs_id(
            &key_json(group_id)?,
            &key_json(epoch)?,
            &serde_json::to_vec(&leaf_index).map_err(|_| StoreError::Serialization)?,
        );
        let value = serde_json::to_vec(key_pairs).map_err(|_| StoreError::Serialization)?;
        self.raw_put(&storage_key(EPOCH_KEY_PAIRS_LABEL, &key), &value)
    }

    fn write_key_package<
        HashReference: store_traits::HashReference<CURRENT_VERSION>,
        KeyPackage: store_traits::KeyPackage<CURRENT_VERSION>,
    >(
        &self,
        hash_ref: &HashReference,
        key_package: &KeyPackage,
    ) -> Result<(), Self::Error> {
        let key = key_json(hash_ref)?;
        self.entity_put(KEY_PACKAGE_LABEL, &key, key_package)
    }

    fn write_psk<
        PskId: store_traits::PskId<CURRENT_VERSION>,
        PskBundle: store_traits::PskBundle<CURRENT_VERSION>,
    >(
        &self,
        psk_id: &PskId,
        psk: &PskBundle,
    ) -> Result<(), Self::Error> {
        let key = key_json(psk_id)?;
        self.entity_put(PSK_LABEL, &key, psk)
    }

    fn mls_group_join_config<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        MlsGroupJoinConfig: store_traits::MlsGroupJoinConfig<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<MlsGroupJoinConfig>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(JOIN_CONFIG_LABEL, &key)
    }

    fn own_leaf_nodes<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        LeafNode: store_traits::LeafNode<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Vec<LeafNode>, Self::Error> {
        let key = key_json(group_id)?;
        self.list_get(OWN_LEAF_NODES_LABEL, &key)
    }

    fn queued_proposal_refs<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ProposalRef: store_traits::ProposalRef<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Vec<ProposalRef>, Self::Error> {
        let key = key_json(group_id)?;
        self.list_get(PROPOSAL_QUEUE_REFS_LABEL, &key)
    }

    fn queued_proposals<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ProposalRef: store_traits::ProposalRef<CURRENT_VERSION>,
        QueuedProposal: store_traits::QueuedProposal<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Vec<(ProposalRef, QueuedProposal)>, Self::Error> {
        let key = key_json(group_id)?;
        let refs: Vec<ProposalRef> = self.list_get(PROPOSAL_QUEUE_REFS_LABEL, &key)?;
        refs.into_iter()
            .map(|proposal_ref| {
                let key = serde_json::to_vec(&(&group_id, &proposal_ref))
                    .map_err(|_| StoreError::Serialization)?;
                let plain = self
                    .raw_get(&storage_key(QUEUED_PROPOSAL_LABEL, &key))?
                    .ok_or(StoreError::Serialization)?;
                let proposal =
                    serde_json::from_slice(&plain).map_err(|_| StoreError::Serialization)?;
                Ok((proposal_ref, proposal))
            })
            .collect()
    }

    fn tree<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        TreeSync: store_traits::TreeSync<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<TreeSync>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(TREE_LABEL, &key)
    }

    fn group_context<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        GroupContext: store_traits::GroupContext<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<GroupContext>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(GROUP_CONTEXT_LABEL, &key)
    }

    fn interim_transcript_hash<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        InterimTranscriptHash: store_traits::InterimTranscriptHash<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<InterimTranscriptHash>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(INTERIM_TRANSCRIPT_HASH_LABEL, &key)
    }

    fn confirmation_tag<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ConfirmationTag: store_traits::ConfirmationTag<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<ConfirmationTag>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(CONFIRMATION_TAG_LABEL, &key)
    }

    fn group_state<
        GroupState: store_traits::GroupState<CURRENT_VERSION>,
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<GroupState>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(GROUP_STATE_LABEL, &key)
    }

    fn message_secrets<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        MessageSecrets: store_traits::MessageSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<MessageSecrets>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(MESSAGE_SECRETS_LABEL, &key)
    }

    fn resumption_psk_store<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ResumptionPskStore: store_traits::ResumptionPskStore<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<ResumptionPskStore>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(RESUMPTION_PSK_STORE_LABEL, &key)
    }

    fn own_leaf_index<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        LeafNodeIndex: store_traits::LeafNodeIndex<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<LeafNodeIndex>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(OWN_LEAF_NODE_INDEX_LABEL, &key)
    }

    fn group_epoch_secrets<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        GroupEpochSecrets: store_traits::GroupEpochSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<GroupEpochSecrets>, Self::Error> {
        let key = key_json(group_id)?;
        self.entity_get(EPOCH_SECRETS_LABEL, &key)
    }

    fn signature_key_pair<
        SignaturePublicKey: store_traits::SignaturePublicKey<CURRENT_VERSION>,
        SignatureKeyPair: store_traits::SignatureKeyPair<CURRENT_VERSION>,
    >(
        &self,
        public_key: &SignaturePublicKey,
    ) -> Result<Option<SignatureKeyPair>, Self::Error> {
        let key = key_json(public_key)?;
        self.entity_get(SIGNATURE_KEY_PAIR_LABEL, &key)
    }

    fn encryption_key_pair<
        HpkeKeyPair: store_traits::HpkeKeyPair<CURRENT_VERSION>,
        EncryptionKey: store_traits::EncryptionKey<CURRENT_VERSION>,
    >(
        &self,
        public_key: &EncryptionKey,
    ) -> Result<Option<HpkeKeyPair>, Self::Error> {
        let key = key_json(public_key)?;
        self.entity_get(ENCRYPTION_KEY_PAIR_LABEL, &key)
    }

    fn encryption_epoch_key_pairs<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        EpochKey: store_traits::EpochKey<CURRENT_VERSION>,
        HpkeKeyPair: store_traits::HpkeKeyPair<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        epoch: &EpochKey,
        leaf_index: u32,
    ) -> Result<Vec<HpkeKeyPair>, Self::Error> {
        let key = epoch_pairs_id(
            &key_json(group_id)?,
            &key_json(epoch)?,
            &serde_json::to_vec(&leaf_index).map_err(|_| StoreError::Serialization)?,
        );
        self.raw_get(&storage_key(EPOCH_KEY_PAIRS_LABEL, &key))?
            .map(|plain| {
                serde_json::from_slice::<Vec<HpkeKeyPair>>(&plain)
                    .map_err(|_| StoreError::Serialization)
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn key_package<
        KeyPackageRef: store_traits::HashReference<CURRENT_VERSION>,
        KeyPackage: store_traits::KeyPackage<CURRENT_VERSION>,
    >(
        &self,
        hash_ref: &KeyPackageRef,
    ) -> Result<Option<KeyPackage>, Self::Error> {
        let key = key_json(hash_ref)?;
        self.entity_get(KEY_PACKAGE_LABEL, &key)
    }

    fn psk<
        PskBundle: store_traits::PskBundle<CURRENT_VERSION>,
        PskId: store_traits::PskId<CURRENT_VERSION>,
    >(
        &self,
        psk_id: &PskId,
    ) -> Result<Option<PskBundle>, Self::Error> {
        let key = key_json(psk_id)?;
        self.entity_get(PSK_LABEL, &key)
    }

    fn remove_proposal<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ProposalRef: store_traits::ProposalRef<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        proposal_ref: &ProposalRef,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        let item = serde_json::to_vec(proposal_ref).map_err(|_| StoreError::Serialization)?;
        self.list_update(PROPOSAL_QUEUE_REFS_LABEL, &key, &item, false)?;
        let key =
            serde_json::to_vec(&(group_id, proposal_ref)).map_err(|_| StoreError::Serialization)?;
        self.kv_del(&storage_key(QUEUED_PROPOSAL_LABEL, &key))
    }

    fn delete_own_leaf_nodes<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(OWN_LEAF_NODES_LABEL, &key))
    }

    fn delete_group_config<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(JOIN_CONFIG_LABEL, &key))
    }

    fn delete_tree<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(TREE_LABEL, &key))
    }

    fn delete_confirmation_tag<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(CONFIRMATION_TAG_LABEL, &key))
    }

    fn delete_group_state<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(GROUP_STATE_LABEL, &key))
    }

    fn delete_context<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(GROUP_CONTEXT_LABEL, &key))
    }

    fn delete_interim_transcript_hash<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(INTERIM_TRANSCRIPT_HASH_LABEL, &key))
    }

    fn delete_message_secrets<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(MESSAGE_SECRETS_LABEL, &key))
    }

    fn delete_all_resumption_psk_secrets<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(RESUMPTION_PSK_STORE_LABEL, &key))
    }

    fn delete_own_leaf_index<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(OWN_LEAF_NODE_INDEX_LABEL, &key))
    }

    fn delete_group_epoch_secrets<GroupId: store_traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        self.kv_del(&storage_key(EPOCH_SECRETS_LABEL, &key))
    }

    fn clear_proposal_queue<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        ProposalRef: store_traits::ProposalRef<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        let key = key_json(group_id)?;
        let refs: Vec<ProposalRef> = self.list_get(PROPOSAL_QUEUE_REFS_LABEL, &key)?;
        for proposal_ref in refs {
            let key = serde_json::to_vec(&(&group_id, &proposal_ref))
                .map_err(|_| StoreError::Serialization)?;
            self.kv_del(&storage_key(QUEUED_PROPOSAL_LABEL, &key))?;
        }
        self.kv_del(&storage_key(PROPOSAL_QUEUE_REFS_LABEL, &key))
    }

    fn delete_signature_key_pair<
        SignaturePublicKey: store_traits::SignaturePublicKey<CURRENT_VERSION>,
    >(
        &self,
        public_key: &SignaturePublicKey,
    ) -> Result<(), Self::Error> {
        let key = key_json(public_key)?;
        self.kv_del(&storage_key(SIGNATURE_KEY_PAIR_LABEL, &key))
    }

    fn delete_encryption_key_pair<EncryptionKey: store_traits::EncryptionKey<CURRENT_VERSION>>(
        &self,
        public_key: &EncryptionKey,
    ) -> Result<(), Self::Error> {
        let key = key_json(public_key)?;
        self.kv_del(&storage_key(ENCRYPTION_KEY_PAIR_LABEL, &key))
    }

    fn delete_encryption_epoch_key_pairs<
        GroupId: store_traits::GroupId<CURRENT_VERSION>,
        EpochKey: store_traits::EpochKey<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        epoch: &EpochKey,
        leaf_index: u32,
    ) -> Result<(), Self::Error> {
        let key = epoch_pairs_id(
            &key_json(group_id)?,
            &key_json(epoch)?,
            &serde_json::to_vec(&leaf_index).map_err(|_| StoreError::Serialization)?,
        );
        self.kv_del(&storage_key(EPOCH_KEY_PAIRS_LABEL, &key))
    }

    fn delete_key_package<KeyPackageRef: store_traits::HashReference<CURRENT_VERSION>>(
        &self,
        hash_ref: &KeyPackageRef,
    ) -> Result<(), Self::Error> {
        let key = key_json(hash_ref)?;
        self.kv_del(&storage_key(KEY_PACKAGE_LABEL, &key))
    }

    fn delete_psk<PskKey: store_traits::PskId<CURRENT_VERSION>>(
        &self,
        psk_id: &PskKey,
    ) -> Result<(), Self::Error> {
        let key = key_json(psk_id)?;
        self.kv_del(&storage_key(PSK_LABEL, &key))
    }
}

/// `OpenMLS` provider pairing `RustCrypto` with the encrypted `SQLite` store.
/// Neither this type nor its key material is clonable or debug-printable
/// beyond a redacted summary.
pub struct PersistProvider {
    crypto: RustCrypto,
    store: EncryptedStore,
}

impl std::fmt::Debug for PersistProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistProvider")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl PersistProvider {
    /// Open (or create) the encrypted MLS store at `path` under `password`.
    /// See [`EncryptedStore::open`] for the fail-closed behavior.
    ///
    /// # Errors
    ///
    /// Forwards [`StoreError`] from [`EncryptedStore::open`].
    pub fn open(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, StoreError> {
        Ok(Self {
            crypto: RustCrypto::default(),
            store: EncryptedStore::open(path, password)?,
        })
    }

    /// The file's non-secret salt; see [`EncryptedStore::salt`].
    #[must_use]
    pub fn salt(&self) -> [u8; SALT_LEN] {
        self.store.salt()
    }
}

impl OpenMlsProvider for PersistProvider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = EncryptedStore;

    fn storage(&self) -> &Self::StorageProvider {
        &self.store
    }

    fn crypto(&self) -> &Self::CryptoProvider {
        &self.crypto
    }

    fn rand(&self) -> &Self::RandProvider {
        &self.crypto
    }
}

/// Independent synthetic installation on an encrypted store; storage is
/// durable (unlike [`crate::mls::MemoryInstallation`]) but the trust model is
/// identical: fresh random scope-local labels, no account binding.
/// Neither this type nor its key material is clonable or debug-printable.
pub struct PersistInstallation {
    provider: PersistProvider,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
}

impl PersistInstallation {
    /// Generate fresh installation keys and a random synthetic scoped identity
    /// on an already-opened store.
    ///
    /// # Errors
    ///
    /// Returns `Operation` if key/identity generation fails.
    pub fn generate(provider: PersistProvider) -> Result<Self, MlsError> {
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

    /// Generate a signed, one-time MLS key package; the private bundle stays
    /// sealed in the encrypted store.
    ///
    /// # Errors
    ///
    /// Returns `Operation` on provider, storage, or encoding failure.
    pub fn key_package(&self) -> Result<Vec<u8>, MlsError> {
        KeyPackage::builder()
            .build(SUITE, &self.provider, &self.signer, self.credential.clone())
            .map_err(|_| MlsError::Operation)?
            .key_package()
            .tls_serialize_detached()
            .map_err(|_| MlsError::Operation)
    }

    fn persist_state(&self, group: &MlsGroup) -> Result<(), MlsError> {
        self.provider
            .store
            .save_app_state(
                &self.signer,
                self.credential.credential.serialized_content(),
                &self.signer.to_public_vec(),
                group.group_id().as_slice(),
            )
            .map_err(|_| MlsError::Operation)
    }

    /// Create a fresh group with an upstream-random group ID and the locked
    /// suite, remembering this installation as the file's active group.
    ///
    /// # Errors
    ///
    /// Returns `Operation` on MLS creation or persistence failure.
    pub fn create_group(self) -> Result<PersistSession, MlsError> {
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
        self.persist_state(&group)?;
        Ok(PersistSession {
            installation: self,
            group,
        })
    }

    /// Join a synthetic trusted Welcome, remembering this installation as the
    /// file's active group. Consumes the installation even on failure;
    /// retry/lifecycle design is pending.
    ///
    /// # Errors
    ///
    /// Rejects invalid messages, unsupported suites, or an MLS join failure.
    pub fn join(self, wire: &[u8]) -> Result<PersistSession, MlsError> {
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
        self.persist_state(&group)?;
        Ok(PersistSession {
            installation: self,
            group,
        })
    }
}

/// One installation's persisted group state. Behaves like
/// [`crate::mls::Session`] but survives provider drops via
/// [`PersistSession::reopen`].
pub struct PersistSession {
    installation: PersistInstallation,
    group: MlsGroup,
}

impl PersistSession {
    /// Reload the file's active group after a full provider drop: restores the
    /// signer, scoped identity, and group from the sealed store. A wrong
    /// password or tampered bytes fails here with `Operation` — fail closed.
    ///
    /// # Errors
    ///
    /// Returns `Operation` when app state is missing/unsealable or the group
    /// fails to load from storage.
    pub fn reopen(provider: PersistProvider) -> Result<Self, MlsError> {
        let (signer, credential, group_id) = provider
            .store
            .load_app_state()
            .map_err(|_| MlsError::Operation)?;
        let group = MlsGroup::load(provider.storage(), &GroupId::from_slice(&group_id))
            .map_err(|_| MlsError::Operation)?
            .ok_or(MlsError::Operation)?;
        Ok(Self {
            installation: PersistInstallation {
                provider,
                signer,
                credential,
            },
            group,
        })
    }

    /// Remove the member with this exact synthetic scoped label and merge locally.
    ///
    /// # Errors
    ///
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
    ///
    /// # Errors
    ///
    /// Rejects malformed packages, unsupported suites, and invalid MLS operations.
    pub fn add(&mut self, packages: &[Vec<u8>]) -> Result<crate::mls::Addition, MlsError> {
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
        let addition = crate::mls::Addition {
            commit: commit.to_bytes().map_err(|_| MlsError::Operation)?,
            welcome: welcome.to_bytes().map_err(|_| MlsError::Operation)?,
        };
        self.group
            .merge_pending_commit(provider)
            .map_err(|_| MlsError::Operation)?;
        Ok(addition)
    }

    /// Rotate this installation's leaf keys (MLS self-update) and merge locally.
    ///
    /// # Errors
    ///
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
    ///
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
    ///
    /// # Errors
    ///
    /// Rejects malformed, foreign-group, unauthenticated, or unexpected messages.
    pub fn receive(&mut self, wire: &[u8]) -> Result<crate::mls::Received, MlsError> {
        let message = MlsMessageIn::tls_deserialize_exact(wire)
            .map_err(|_| MlsError::InvalidInput)?
            .try_into_protocol_message()
            .map_err(|_| MlsError::InvalidInput)?;
        let Ok(processed) = self
            .group
            .process_message(&self.installation.provider, message)
        else {
            // Same reconciliation as `Session::receive`: reload the provider's
            // latest sealed state, never an earlier snapshot.
            self.group =
                MlsGroup::load(self.installation.provider.storage(), self.group.group_id())
                    .map_err(|_| MlsError::Operation)?
                    .ok_or(MlsError::Operation)?;
            return Err(MlsError::Operation);
        };
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(message) => {
                Ok(crate::mls::Received::Application(message.into_bytes()))
            }
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                self.group
                    .merge_staged_commit(&self.installation.provider, *commit)
                    .map_err(|_| MlsError::Operation)?;
                Ok(crate::mls::Received::EpochChanged)
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
}
