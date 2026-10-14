//! Node types for the overlay (in memory) and the committed tree (in the CAS).

use sandbox_types::hash::ContentHash;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A node in the mutable overlay. File bytes stay in memory until `commit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Directory,
    File(Vec<u8>),
    Symlink(String),
}

/// A committed node, stored in the CAS as JSON. The node's own hash is the
/// hash of that JSON, so a directory's hash covers its whole subtree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stored {
    /// Child name to the hash of the child's `Stored` node.
    Directory {
        entries: BTreeMap<String, ContentHash>,
    },
    /// `content` is the hash of the file bytes, stored as a separate blob.
    File {
        content: ContentHash,
        size: u64,
    },
    Symlink(String),
}
