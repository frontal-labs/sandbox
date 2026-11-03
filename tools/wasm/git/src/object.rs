//! Git objects: hashing, the loose object store, and the tree and commit formats.

use crate::Fatal;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use sha1::{Digest, Sha1};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

pub type Oid = [u8; 20];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Blob,
    Tree,
    Commit,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Blob => "blob",
            Kind::Tree => "tree",
            Kind::Commit => "commit",
        }
    }
}

/// The object id of `body` as a `kind` object, which is what git hashes.
pub fn hash_object(kind: Kind, body: &[u8]) -> Oid {
    let mut hasher = Sha1::new();
    hasher.update(format!("{} {}\0", kind.name(), body.len()).as_bytes());
    hasher.update(body);
    hasher.finalize().into()
}

pub fn hex(oid: &Oid) -> String {
    oid.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn parse_hex(text: &str) -> Option<Oid> {
    if text.len() != 40 || !text.is_ascii() {
        return None;
    }
    let mut oid = [0u8; 20];
    for (slot, pair) in oid.iter_mut().zip(text.as_bytes().chunks(2)) {
        let digits = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(digits, 16).ok()?;
    }
    Some(oid)
}

/// The loose object store under `.git/objects`.
pub struct ObjectStore {
    objects: PathBuf,
}

impl ObjectStore {
    pub fn new(git_dir: &std::path::Path) -> Self {
        ObjectStore {
            objects: git_dir.join("objects"),
        }
    }

    fn path(&self, oid: &Oid) -> PathBuf {
        let hex = hex(oid);
        self.objects.join(&hex[..2]).join(&hex[2..])
    }

    pub fn write(&self, kind: Kind, body: &[u8]) -> Result<Oid, Fatal> {
        let oid = hash_object(kind, body);
        let path = self.path(&oid);
        if path.exists() {
            return Ok(oid);
        }
        let Some(dir) = path.parent() else {
            return Err(Fatal::new("object path has no directory"));
        };
        fs::create_dir_all(dir).map_err(|e| Fatal::new(format!("cannot create {}: {e}", dir.display())))?;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(format!("{} {}\0", kind.name(), body.len()).as_bytes())
            .and_then(|()| encoder.write_all(body))
            .map_err(|e| Fatal::new(format!("cannot compress object: {e}")))?;
        let compressed = encoder
            .finish()
            .map_err(|e| Fatal::new(format!("cannot compress object: {e}")))?;
        fs::write(&path, compressed)
            .map_err(|e| Fatal::new(format!("cannot write {}: {e}", path.display())))?;
        Ok(oid)
    }

    pub fn read(&self, oid: &Oid) -> Result<(Kind, Vec<u8>), Fatal> {
        let path = self.path(oid);
        let compressed = fs::read(&path)
            .map_err(|_| Fatal::new(format!("bad object {}", hex(oid))))?;
        let mut raw = Vec::new();
        ZlibDecoder::new(compressed.as_slice())
            .read_to_end(&mut raw)
            .map_err(|e| Fatal::new(format!("corrupt object {}: {e}", hex(oid))))?;
        let nul = raw
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| Fatal::new(format!("corrupt object {}", hex(oid))))?;
        let header = std::str::from_utf8(&raw[..nul])
            .map_err(|_| Fatal::new(format!("corrupt object {}", hex(oid))))?;
        let kind = match header.split(' ').next() {
            Some("blob") => Kind::Blob,
            Some("tree") => Kind::Tree,
            Some("commit") => Kind::Commit,
            _ => return Err(Fatal::new(format!("unsupported object {}", hex(oid)))),
        };
        Ok((kind, raw[nul + 1..].to_vec()))
    }
}

/// One entry of a tree object. Trees list their entries sorted as git sorts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: u32,
    pub name: Vec<u8>,
    pub oid: Oid,
}

impl TreeEntry {
    pub fn is_tree(&self) -> bool {
        self.mode == 0o40000
    }
}

pub fn encode_tree(entries: &[TreeEntry]) -> Vec<u8> {
    let mut body = Vec::new();
    for entry in entries {
        body.extend_from_slice(format!("{:o} ", entry.mode).as_bytes());
        body.extend_from_slice(&entry.name);
        body.push(0);
        body.extend_from_slice(&entry.oid);
    }
    body
}

pub fn parse_tree(body: &[u8]) -> Result<Vec<TreeEntry>, Fatal> {
    let corrupt = || Fatal::new("corrupt tree object");
    let mut entries = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let space = rest.iter().position(|b| *b == b' ').ok_or_else(corrupt)?;
        let nul = rest.iter().position(|b| *b == 0).ok_or_else(corrupt)?;
        if nul < space || rest.len() < nul + 21 {
            return Err(corrupt());
        }
        let mode_text = std::str::from_utf8(&rest[..space]).map_err(|_| corrupt())?;
        let mode = u32::from_str_radix(mode_text, 8).map_err(|_| corrupt())?;
        let mut oid = [0u8; 20];
        oid.copy_from_slice(&rest[nul + 1..nul + 21]);
        entries.push(TreeEntry {
            mode,
            name: rest[space + 1..nul].to_vec(),
            oid,
        });
        rest = &rest[nul + 21..];
    }
    Ok(entries)
}

/// Sorts tree entries as git does: a directory sorts as if its name ended in `/`.
pub fn sort_tree_entries(entries: &mut [TreeEntry]) {
    entries.sort_by_key(sort_key);
}

fn sort_key(entry: &TreeEntry) -> Vec<u8> {
    let mut key = entry.name.clone();
    if entry.is_tree() {
        key.push(b'/');
    }
    key
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub tree: Oid,
    pub parents: Vec<Oid>,
    /// `name <email> seconds offset`, as git writes it.
    pub author: String,
    pub committer: String,
    pub message: String,
}

pub fn encode_commit(commit: &Commit) -> Vec<u8> {
    let mut text = format!("tree {}\n", hex(&commit.tree));
    for parent in &commit.parents {
        text.push_str(&format!("parent {}\n", hex(parent)));
    }
    text.push_str(&format!("author {}\n", commit.author));
    text.push_str(&format!("committer {}\n\n", commit.committer));
    text.push_str(&commit.message);
    text.into_bytes()
}

pub fn parse_commit(body: &[u8]) -> Result<Commit, Fatal> {
    let corrupt = || Fatal::new("corrupt commit object");
    let text = std::str::from_utf8(body).map_err(|_| corrupt())?;
    let (headers, message) = text.split_once("\n\n").ok_or_else(corrupt)?;
    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    for line in headers.lines() {
        match line.split_once(' ') {
            Some(("tree", value)) => tree = parse_hex(value),
            Some(("parent", value)) => parents.push(parse_hex(value).ok_or_else(corrupt)?),
            Some(("author", value)) => author = Some(value.to_string()),
            Some(("committer", value)) => committer = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(Commit {
        tree: tree.ok_or_else(corrupt)?,
        parents,
        author: author.ok_or_else(corrupt)?,
        committer: committer.ok_or_else(corrupt)?,
        message: message.to_string(),
    })
}
