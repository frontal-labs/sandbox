use super::invalid;
use crate::v1::types as pb;
use sandbox_types::SandboxError;
use sandbox_types::{SessionId, SnapshotId, TenantId};
use ulid::Ulid;

fn parse(value: &str, field: &str) -> Result<Ulid, SandboxError> {
    Ulid::from_string(value).map_err(|_| invalid(format!("{field}: not a valid ULID: {value:?}")))
}

impl From<SessionId> for pb::SessionId {
    fn from(id: SessionId) -> Self {
        Self {
            value: id.as_ulid().to_string(),
        }
    }
}

impl TryFrom<pb::SessionId> for SessionId {
    type Error = SandboxError;
    fn try_from(id: pb::SessionId) -> Result<Self, Self::Error> {
        Ok(SessionId::from_ulid(parse(&id.value, "session_id")?))
    }
}

impl From<SnapshotId> for pb::SnapshotId {
    fn from(id: SnapshotId) -> Self {
        Self {
            value: id.as_ulid().to_string(),
        }
    }
}

impl TryFrom<pb::SnapshotId> for SnapshotId {
    type Error = SandboxError;
    fn try_from(id: pb::SnapshotId) -> Result<Self, Self::Error> {
        Ok(SnapshotId::from_ulid(parse(&id.value, "snapshot_id")?))
    }
}

impl From<TenantId> for pb::TenantId {
    fn from(id: TenantId) -> Self {
        Self {
            value: id.as_ulid().to_string(),
        }
    }
}

impl TryFrom<pb::TenantId> for TenantId {
    type Error = SandboxError;
    fn try_from(id: pb::TenantId) -> Result<Self, Self::Error> {
        Ok(TenantId::from_ulid(parse(&id.value, "tenant_id")?))
    }
}
