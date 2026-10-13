use blake3::Hash as Blake3Hash;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

pub struct ContentHash(pub Blake3Hash);

impl ContentHash {
    pub fn new(data: &[u8]) -> Self {
        Self(blake3::hash(data))
    }

    pub fn from_hash(hash: Blake3Hash) -> Self {
        Self(hash)
    }

    pub fn as_hash(&self) -> &Blake3Hash {
        &self.0
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    pub fn to_hex(&self) -> String {
        self.0.to_hex().to_string()
    }

    pub fn from_hex(s: &str) -> Result<Self, blake3::HexError> {
        Ok(Self(Blake3Hash::from_hex(s)?))
    }

    pub const fn nil() -> Self {
        Self(Blake3Hash::from_bytes([0; 32]))
    }
}

impl Clone for ContentHash {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl Copy for ContentHash {}

impl PartialEq for ContentHash {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_bytes() == other.0.as_bytes()
    }
}

impl Eq for ContentHash {}

impl PartialOrd for ContentHash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ContentHash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_bytes().cmp(other.0.as_bytes())
    }
}

impl std::hash::Hash for ContentHash {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_bytes().hash(state);
    }
}

impl Serialize for ContentHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ContentHash::from_hex(&s).map_err(de::Error::custom)
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.to_hex())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl FromStr for ContentHash {
    type Err = blake3::HexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl From<Blake3Hash> for ContentHash {
    fn from(hash: Blake3Hash) -> Self {
        Self(hash)
    }
}

impl From<ContentHash> for Blake3Hash {
    fn from(hash: ContentHash) -> Blake3Hash {
        hash.0
    }
}

impl AsRef<[u8]> for ContentHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}