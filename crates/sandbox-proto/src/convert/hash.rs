use super::invalid;
use crate::v1::types as pb;
use sandbox_types::hash::ContentHash;
use sandbox_types::SandboxError;

impl From<ContentHash> for pb::ContentHash {
    fn from(hash: ContentHash) -> Self {
        Self {
            value: hash.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<pb::ContentHash> for ContentHash {
    type Error = SandboxError;
    fn try_from(hash: pb::ContentHash) -> Result<Self, Self::Error> {
        let bytes: [u8; 32] = hash.value.as_slice().try_into().map_err(|_| {
            invalid(format!(
                "content hash must be 32 bytes, got {}",
                hash.value.len()
            ))
        })?;
        Ok(ContentHash::from_hash(blake3::Hash::from_bytes(bytes)))
    }
}
