use super::invalid;
use super::time::{from_timestamp, to_timestamp};
use crate::v1::types as pb;
use sandbox_types::event::{Event, FileOp};
use sandbox_types::hash::ContentHash;
use sandbox_types::session::Tier;
use sandbox_types::{SandboxError, SessionId, SnapshotId, TenantId};

fn req<T>(value: Option<T>, field: &str) -> Result<T, SandboxError> {
    value.ok_or_else(|| invalid(format!("{field} is required")))
}

fn file_op_to_pb(op: FileOp) -> pb::FileOp {
    match op {
        FileOp::Create => pb::FileOp::Create,
        FileOp::Write => pb::FileOp::Write,
        FileOp::Delete => pb::FileOp::Delete,
        FileOp::Rename => pb::FileOp::Rename,
        FileOp::Mkdir => pb::FileOp::Mkdir,
        FileOp::Symlink => pb::FileOp::Symlink,
    }
}

fn file_op_from_pb(op: i32) -> Result<FileOp, SandboxError> {
    match pb::FileOp::try_from(op).map_err(|_| invalid(format!("unknown file op {op}")))? {
        pb::FileOp::Unspecified => Err(invalid("file op is unspecified")),
        pb::FileOp::Create => Ok(FileOp::Create),
        pb::FileOp::Write => Ok(FileOp::Write),
        pb::FileOp::Delete => Ok(FileOp::Delete),
        pb::FileOp::Rename => Ok(FileOp::Rename),
        pb::FileOp::Mkdir => Ok(FileOp::Mkdir),
        pb::FileOp::Symlink => Ok(FileOp::Symlink),
    }
}

fn tier_to_pb(tier: Tier) -> i32 {
    pb::Tier::from(tier) as i32
}

fn tier_from_pb(tier: i32) -> Result<Tier, SandboxError> {
    pb::Tier::try_from(tier)
        .map_err(|_| invalid(format!("unknown tier {tier}")))?
        .try_into()
}

fn sid(id: SessionId) -> Option<pb::SessionId> {
    Some(id.into())
}

fn ts(time: std::time::SystemTime) -> Option<prost_types::Timestamp> {
    Some(to_timestamp(time))
}

impl From<Event> for pb::Event {
    fn from(event: Event) -> Self {
        use pb::event::Kind;
        let kind = match event {
            Event::SessionCreated {
                session_id,
                tenant_id,
                tier,
                config_hash,
                timestamp,
            } => Kind::SessionCreated(pb::SessionCreated {
                session_id: sid(session_id),
                tenant_id: Some(tenant_id.into()),
                tier: tier_to_pb(tier),
                config_hash: Some(config_hash.into()),
                timestamp: ts(timestamp),
            }),
            Event::ExecStarted {
                session_id,
                seq,
                argv,
                cwd,
                env_hash,
                timestamp,
            } => Kind::ExecStarted(pb::ExecStarted {
                session_id: sid(session_id),
                seq,
                argv,
                cwd,
                env_hash: Some(env_hash.into()),
                timestamp: ts(timestamp),
            }),
            Event::ExecExited {
                session_id,
                seq,
                exit_code,
                cpu_ms,
                wall_ms,
                timestamp,
            } => Kind::ExecExited(pb::ExecExited {
                session_id: sid(session_id),
                seq,
                exit_code,
                cpu_ms,
                wall_ms,
                timestamp: ts(timestamp),
            }),
            Event::StdoutChunk {
                session_id,
                seq,
                data,
                timestamp,
            } => Kind::StdoutChunk(pb::StdoutChunk {
                session_id: sid(session_id),
                seq,
                data,
                timestamp: ts(timestamp),
            }),
            Event::StderrChunk {
                session_id,
                seq,
                data,
                timestamp,
            } => Kind::StderrChunk(pb::StderrChunk {
                session_id: sid(session_id),
                seq,
                data,
                timestamp: ts(timestamp),
            }),
            Event::FileChanged {
                session_id,
                seq,
                path,
                op,
                before_hash,
                after_hash,
                timestamp,
            } => Kind::FileChanged(pb::FileChanged {
                session_id: sid(session_id),
                seq,
                path,
                op: file_op_to_pb(op) as i32,
                before_hash: before_hash.map(Into::into),
                after_hash: after_hash.map(Into::into),
                timestamp: ts(timestamp),
            }),
            Event::NetRequest {
                session_id,
                seq,
                host,
                method,
                status,
                request_bytes,
                response_bytes,
                timestamp,
            } => Kind::NetRequest(pb::NetRequest {
                session_id: sid(session_id),
                seq,
                host,
                method,
                status: status.map(u32::from),
                request_bytes,
                response_bytes,
                timestamp: ts(timestamp),
            }),
            Event::PolicyDenied {
                session_id,
                seq,
                capability,
                reason,
                timestamp,
            } => Kind::PolicyDenied(pb::PolicyDenied {
                session_id: sid(session_id),
                seq,
                capability,
                reason,
                timestamp: ts(timestamp),
            }),
            Event::SnapshotTaken {
                session_id,
                snapshot_id,
                parent,
                vfs_root,
                timestamp,
            } => Kind::SnapshotTaken(pb::SnapshotTaken {
                session_id: sid(session_id),
                snapshot_id: Some(snapshot_id.into()),
                parent: parent.map(Into::into),
                vfs_root: Some(vfs_root.into()),
                timestamp: ts(timestamp),
            }),
            Event::Promoted {
                session_id,
                from_tier,
                to_tier,
                timestamp,
            } => Kind::Promoted(pb::Promoted {
                session_id: sid(session_id),
                from_tier: tier_to_pb(from_tier),
                to_tier: tier_to_pb(to_tier),
                timestamp: ts(timestamp),
            }),
            Event::Slept {
                session_id,
                snapshot_id,
                timestamp,
            } => Kind::Slept(pb::Slept {
                session_id: sid(session_id),
                snapshot_id: Some(snapshot_id.into()),
                timestamp: ts(timestamp),
            }),
            Event::Woken {
                session_id,
                snapshot_id,
                timestamp,
            } => Kind::Woken(pb::Woken {
                session_id: sid(session_id),
                snapshot_id: Some(snapshot_id.into()),
                timestamp: ts(timestamp),
            }),
            Event::SessionDestroyed {
                session_id,
                timestamp,
            } => Kind::SessionDestroyed(pb::SessionDestroyed {
                session_id: sid(session_id),
                timestamp: ts(timestamp),
            }),
        };
        Self { kind: Some(kind) }
    }
}

impl TryFrom<pb::Event> for Event {
    type Error = SandboxError;
    fn try_from(event: pb::Event) -> Result<Self, Self::Error> {
        use pb::event::Kind;
        let kind = req(event.kind, "event kind")?;
        Ok(match kind {
            Kind::SessionCreated(e) => Event::SessionCreated {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                tenant_id: TenantId::try_from(req(e.tenant_id, "tenant_id")?)?,
                tier: tier_from_pb(e.tier)?,
                config_hash: req(e.config_hash, "config_hash")?.try_into()?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::ExecStarted(e) => Event::ExecStarted {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                seq: e.seq,
                argv: e.argv,
                cwd: e.cwd,
                env_hash: req(e.env_hash, "env_hash")?.try_into()?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::ExecExited(e) => Event::ExecExited {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                seq: e.seq,
                exit_code: e.exit_code,
                cpu_ms: e.cpu_ms,
                wall_ms: e.wall_ms,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::StdoutChunk(e) => Event::StdoutChunk {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                seq: e.seq,
                data: e.data,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::StderrChunk(e) => Event::StderrChunk {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                seq: e.seq,
                data: e.data,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::FileChanged(e) => Event::FileChanged {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                seq: e.seq,
                path: e.path,
                op: file_op_from_pb(e.op)?,
                before_hash: e.before_hash.map(ContentHash::try_from).transpose()?,
                after_hash: e.after_hash.map(ContentHash::try_from).transpose()?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::NetRequest(e) => Event::NetRequest {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                seq: e.seq,
                host: e.host,
                method: e.method,
                status: e
                    .status
                    .map(|s| {
                        u16::try_from(s)
                            .map_err(|_| invalid(format!("http status {s} out of range")))
                    })
                    .transpose()?,
                request_bytes: e.request_bytes,
                response_bytes: e.response_bytes,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::PolicyDenied(e) => Event::PolicyDenied {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                seq: e.seq,
                capability: e.capability,
                reason: e.reason,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::SnapshotTaken(e) => Event::SnapshotTaken {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                snapshot_id: SnapshotId::try_from(req(e.snapshot_id, "snapshot_id")?)?,
                parent: e.parent.map(SnapshotId::try_from).transpose()?,
                vfs_root: req(e.vfs_root, "vfs_root")?.try_into()?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::Promoted(e) => Event::Promoted {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                from_tier: tier_from_pb(e.from_tier)?,
                to_tier: tier_from_pb(e.to_tier)?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::Slept(e) => Event::Slept {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                snapshot_id: SnapshotId::try_from(req(e.snapshot_id, "snapshot_id")?)?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::Woken(e) => Event::Woken {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                snapshot_id: SnapshotId::try_from(req(e.snapshot_id, "snapshot_id")?)?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
            Kind::SessionDestroyed(e) => Event::SessionDestroyed {
                session_id: req(e.session_id, "session_id")?.try_into()?,
                timestamp: from_timestamp(&req(e.timestamp, "timestamp")?)?,
            },
        })
    }
}
