use crate::session::{ExecHandle, Session};
use sandbox_types::{ExecRequest, SandboxError, SessionConfig, SessionId, SnapshotId, Tier};
use async_trait::async_trait;

pub struct IsolateBackend {
    id: SessionId,
    tier: Tier,
}

impl IsolateBackend {
    pub async fn new(_config: SessionConfig) -> Result<Self, SandboxError> {
        Ok(Self {
            id: SessionId::new(),
            tier: Tier::Isolate,
        })
    }
}

#[async_trait]
impl Session for IsolateBackend {
    fn id(&self) -> SessionId {
        self.id
    }

    fn tier(&self) -> Tier {
        self.tier
    }

    async fn exec(&self, _req: ExecRequest) -> Result<ExecHandle, SandboxError> {
        Ok(ExecHandle::default())
    }

    fn fs(&self) -> Box<dyn crate::session::SessionFs> {
        // Placeholder - would return VFS
        unimplemented!()
    }

    async fn snapshot(&self) -> Result<SnapshotId, SandboxError> {
        Ok(SnapshotId::new())
    }

    async fn sleep(&self) -> Result<SnapshotId, SandboxError> {
        Ok(SnapshotId::new())
    }

    async fn destroy(self: Box<Self>) -> Result<(), SandboxError> {
        Ok(())
    }
}

pub async fn restore(_snapshot_id: SnapshotId) -> Result<Box<dyn Session>, SandboxError> {
    Err(SandboxError::unsupported("restore not implemented"))
}