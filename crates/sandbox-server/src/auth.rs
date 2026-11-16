//! API key authentication.
//!
//! Keys look like `sbx_<key-id>_<secret>`. The key id is public and selects
//! the record; the 256-bit secret is never stored. Only its BLAKE3 digest is
//! kept, so a leaked store does not reveal usable keys. The secret has full
//! entropy, so a fast hash is appropriate here; slow password hashing would
//! only add per-request cost.

use sandbox_types::SandboxError;
use std::collections::HashMap;
use std::sync::RwLock;

const KEY_PREFIX: &str = "sbx_";

/// Opaque id of a stored key. Safe to log.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyId(String);

impl KeyId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A key id read back from saved state, which was written by this module.
    pub fn from_stored(id: String) -> Self {
        Self(id)
    }
}

#[derive(Default)]
pub struct AuthManager {
    keys: RwLock<HashMap<String, blake3::Hash>>,
}

impl AuthManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a key and returns its plaintext. The plaintext is not stored
    /// and cannot be recovered; the caller must hand it to the user now.
    pub fn create_key(&self) -> String {
        let mut id_bytes = [0u8; 8];
        let mut secret_bytes = [0u8; 32];
        fill_random(&mut id_bytes);
        fill_random(&mut secret_bytes);

        let id = hex(&id_bytes);
        let secret = hex(&secret_bytes);
        let digest = blake3::hash(secret.as_bytes());

        self.keys
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.clone(), digest);

        format!("{KEY_PREFIX}{id}_{secret}")
    }

    /// Checks a presented key. Returns the key id on success.
    pub fn validate_api_key(&self, api_key: &str) -> Result<KeyId, SandboxError> {
        let denied = || SandboxError::permission_denied("invalid API key");

        let body = api_key.strip_prefix(KEY_PREFIX).ok_or_else(denied)?;
        let (id, secret) = body.split_once('_').ok_or_else(denied)?;

        let keys = self
            .keys
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored = keys.get(id).copied().ok_or_else(denied)?;

        // blake3::Hash equality is constant-time, so a wrong secret leaks no
        // timing information about the stored digest.
        if blake3::hash(secret.as_bytes()) == stored {
            Ok(KeyId(id.to_string()))
        } else {
            Err(denied())
        }
    }

    /// The stored key records as (id, hex digest). Digests only, so the result
    /// cannot be used to authenticate.
    pub fn export_digests(&self) -> Vec<(String, String)> {
        let keys = self
            .keys
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut out: Vec<(String, String)> = keys
            .iter()
            .map(|(id, digest)| (id.clone(), digest.to_hex().to_string()))
            .collect();
        out.sort();
        out
    }

    /// Loads key records exported by `export_digests`. Keys already present are
    /// replaced.
    pub fn import_digests(&self, records: &[(String, String)]) -> Result<(), SandboxError> {
        let mut parsed = Vec::with_capacity(records.len());
        for (id, digest) in records {
            let digest = blake3::Hash::from_hex(digest)
                .map_err(|_| SandboxError::internal(format!("key {id} has a damaged digest")))?;
            parsed.push((id.clone(), digest));
        }
        let mut keys = self
            .keys
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        keys.extend(parsed);
        Ok(())
    }

    /// Revokes a key by id. Returns whether a key was removed.
    pub fn revoke(&self, id: &KeyId) -> bool {
        self.keys
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id.as_str())
            .is_some()
    }
}

/// Fills `buf` from the operating system's random source.
///
/// The bytes come from version 4 UUIDs, whose generator reads the OS CSPRNG. The
/// `uuid` crate is already a dependency and is available to the Bazel build,
/// unlike `getrandom`. If the OS source is unavailable, `Uuid::new_v4` panics,
/// so no key is ever minted from a weak source.
fn fill_random(buf: &mut [u8]) {
    // Bytes 6 and 8 of a v4 UUID hold the version and variant bits, so they are
    // not random and are skipped. The other 14 bytes are.
    const RANDOM_BYTES: [usize; 14] = [0, 1, 2, 3, 4, 5, 7, 9, 10, 11, 12, 13, 14, 15];
    let mut filled = 0;
    while filled < buf.len() {
        let uuid = uuid::Uuid::new_v4();
        let bytes = uuid.as_bytes();
        for &i in &RANDOM_BYTES {
            if filled == buf.len() {
                break;
            }
            buf[filled] = bytes[i];
            filled += 1;
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
