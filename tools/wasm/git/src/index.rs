//! The index (`.git/index`, format version 2): the staged tree.

use crate::object::{hex, Oid};
use crate::Fatal;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;

pub const REGULAR_FILE: u32 = 0o100644;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub mode: u32,
    pub oid: Oid,
    pub size: u32,
}

/// Staged entries by path, sorted by path as git sorts them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Index {
    pub entries: BTreeMap<String, IndexEntry>,
}

const HEADER_LEN: usize = 12;
const ENTRY_FIXED_LEN: usize = 62;

impl Index {
    pub fn parse(bytes: &[u8]) -> Result<Index, Fatal> {
        let corrupt = || Fatal::new("index file is corrupt");
        if bytes.len() < HEADER_LEN + 20 || &bytes[..4] != b"DIRC" {
            return Err(corrupt());
        }
        let version = be32(&bytes[4..8]).ok_or_else(corrupt)?;
        if version != 2 {
            return Err(Fatal::new(format!("unsupported index version {version}")));
        }
        let count = be32(&bytes[8..12]).ok_or_else(corrupt)? as usize;
        let body_end = bytes.len() - 20;
        if Sha1::digest(&bytes[..body_end]).as_slice() != &bytes[body_end..] {
            return Err(Fatal::new("index file checksum does not match"));
        }

        let mut entries = BTreeMap::new();
        let mut at = HEADER_LEN;
        for _ in 0..count {
            let fixed = bytes.get(at..at + ENTRY_FIXED_LEN).ok_or_else(corrupt)?;
            let mode = be32(&fixed[24..28]).ok_or_else(corrupt)?;
            let size = be32(&fixed[36..40]).ok_or_else(corrupt)?;
            let mut oid = [0u8; 20];
            oid.copy_from_slice(&fixed[40..60]);
            let flags = u16::from_be_bytes([fixed[60], fixed[61]]);
            if flags & 0x4000 != 0 {
                return Err(Fatal::new("extended index entries are not supported"));
            }
            let name_start = at + ENTRY_FIXED_LEN;
            let name_len = usize::from(flags & 0x0fff);
            let name_end = if name_len < 0x0fff {
                name_start + name_len
            } else {
                name_start
                    + bytes[name_start..]
                        .iter()
                        .position(|b| *b == 0)
                        .ok_or_else(corrupt)?
            };
            let name = bytes.get(name_start..name_end).ok_or_else(corrupt)?;
            let name = String::from_utf8(name.to_vec())
                .map_err(|_| Fatal::new("index path is not valid UTF-8"))?;
            // An entry's length, not its offset, is padded to 8 bytes, with one to eight NULs.
            let entry_len = ENTRY_FIXED_LEN + (name_end - name_start);
            at += (entry_len + 8) & !7;
            entries.insert(name, IndexEntry { mode, oid, size });
        }
        Ok(Index { entries })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"DIRC");
        out.extend_from_slice(&2u32.to_be_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_be_bytes());
        for (path, entry) in &self.entries {
            // Times, device, inode, uid, and gid are zero. This tool compares contents,
            // never stat data, so it writes no stat cache. Git rechecks the contents.
            out.extend_from_slice(&[0u8; 24]);
            out.extend_from_slice(&entry.mode.to_be_bytes());
            out.extend_from_slice(&[0u8; 8]);
            out.extend_from_slice(&entry.size.to_be_bytes());
            out.extend_from_slice(&entry.oid);
            let name_len = path.len().min(0x0fff) as u16;
            out.extend_from_slice(&name_len.to_be_bytes());
            out.extend_from_slice(path.as_bytes());
            let entry_len = ENTRY_FIXED_LEN + path.len();
            let padded = (entry_len + 8) & !7;
            out.extend(std::iter::repeat_n(0u8, padded - entry_len));
        }
        let checksum = Sha1::digest(&out);
        out.extend_from_slice(&checksum);
        out
    }
}

fn be32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.try_into().ok()?))
}

/// Lowercase hex, for messages.
pub fn short(oid: &Oid) -> String {
    hex(oid)[..7].to_string()
}
