use crate::hash::ContentHash;
use crate::ids::{SessionId, SnapshotId, TenantId};
use crate::session::Tier;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum Event {
    SessionCreated {
        session_id: SessionId,
        tenant_id: TenantId,
        tier: Tier,
        config_hash: ContentHash,
        timestamp: SystemTime,
    },
    ExecStarted {
        session_id: SessionId,
        seq: u64,
        argv: Vec<String>,
        cwd: String,
        env_hash: ContentHash,
        timestamp: SystemTime,
    },
    ExecExited {
        session_id: SessionId,
        seq: u64,
        exit_code: i32,
        cpu_ms: u64,
        wall_ms: u64,
        timestamp: SystemTime,
    },
    StdoutChunk {
        session_id: SessionId,
        seq: u64,
        data: Vec<u8>,
        timestamp: SystemTime,
    },
    StderrChunk {
        session_id: SessionId,
        seq: u64,
        data: Vec<u8>,
        timestamp: SystemTime,
    },
    FileChanged {
        session_id: SessionId,
        seq: u64,
        path: String,
        op: FileOp,
        before_hash: Option<ContentHash>,
        after_hash: Option<ContentHash>,
        timestamp: SystemTime,
    },
    NetRequest {
        session_id: SessionId,
        seq: u64,
        host: String,
        method: String,
        status: Option<u16>,
        request_bytes: u64,
        response_bytes: u64,
        timestamp: SystemTime,
    },
    PolicyDenied {
        session_id: SessionId,
        seq: u64,
        capability: String,
        reason: String,
        timestamp: SystemTime,
    },
    SnapshotTaken {
        session_id: SessionId,
        snapshot_id: SnapshotId,
        parent: Option<SnapshotId>,
        vfs_root: ContentHash,
        timestamp: SystemTime,
    },
    Promoted {
        session_id: SessionId,
        from_tier: Tier,
        to_tier: Tier,
        timestamp: SystemTime,
    },
    Slept {
        session_id: SessionId,
        snapshot_id: SnapshotId,
        timestamp: SystemTime,
    },
    Woken {
        session_id: SessionId,
        snapshot_id: SnapshotId,
        timestamp: SystemTime,
    },
    SessionDestroyed {
        session_id: SessionId,
        timestamp: SystemTime,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOp {
    Create,
    Write,
    Delete,
    Rename,
    Mkdir,
    Symlink,
}

impl Event {
    pub fn session_id(&self) -> SessionId {
        match self {
            Event::SessionCreated { session_id, .. } => *session_id,
            Event::ExecStarted { session_id, .. } => *session_id,
            Event::ExecExited { session_id, .. } => *session_id,
            Event::StdoutChunk { session_id, .. } => *session_id,
            Event::StderrChunk { session_id, .. } => *session_id,
            Event::FileChanged { session_id, .. } => *session_id,
            Event::NetRequest { session_id, .. } => *session_id,
            Event::PolicyDenied { session_id, .. } => *session_id,
            Event::SnapshotTaken { session_id, .. } => *session_id,
            Event::Promoted { session_id, .. } => *session_id,
            Event::Slept { session_id, .. } => *session_id,
            Event::Woken { session_id, .. } => *session_id,
            Event::SessionDestroyed { session_id, .. } => *session_id,
        }
    }

    pub fn seq(&self) -> Option<u64> {
        match self {
            Event::ExecStarted { seq, .. } => Some(*seq),
            Event::ExecExited { seq, .. } => Some(*seq),
            Event::StdoutChunk { seq, .. } => Some(*seq),
            Event::StderrChunk { seq, .. } => Some(*seq),
            Event::FileChanged { seq, .. } => Some(*seq),
            Event::NetRequest { seq, .. } => Some(*seq),
            Event::PolicyDenied { seq, .. } => Some(*seq),
            _ => None,
        }
    }

    pub fn timestamp(&self) -> SystemTime {
        match self {
            Event::SessionCreated { timestamp, .. } => *timestamp,
            Event::ExecStarted { timestamp, .. } => *timestamp,
            Event::ExecExited { timestamp, .. } => *timestamp,
            Event::StdoutChunk { timestamp, .. } => *timestamp,
            Event::StderrChunk { timestamp, .. } => *timestamp,
            Event::FileChanged { timestamp, .. } => *timestamp,
            Event::NetRequest { timestamp, .. } => *timestamp,
            Event::PolicyDenied { timestamp, .. } => *timestamp,
            Event::SnapshotTaken { timestamp, .. } => *timestamp,
            Event::Promoted { timestamp, .. } => *timestamp,
            Event::Slept { timestamp, .. } => *timestamp,
            Event::Woken { timestamp, .. } => *timestamp,
            Event::SessionDestroyed { timestamp, .. } => *timestamp,
        }
    }
}