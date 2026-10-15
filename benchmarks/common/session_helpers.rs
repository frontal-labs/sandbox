use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_session::{IsolateBackend, Session, SessionConfig, Tier};
use sandbox_types::SessionId;
use std::sync::Arc;

pub struct SessionBenchHarness {
    pub session: Box<dyn Session>,
    pub ctx: BenchmarkContext,
}

impl SessionBenchHarness {
    pub async fn new_isolate(ctx: &BenchmarkContext) -> Result<Self> {
        let config = SessionConfig::default().with_tier(Tier::Isolate);
        let session = IsolateBackend::new(config).await?;
        
        Ok(Self {
            session,
            ctx: BenchmarkContext::new()?,
        })
    }

    pub async fn new_auto(ctx: &BenchmarkContext) -> Result<Self> {
        let config = SessionConfig::default().with_tier(Tier::Auto);
        let session = IsolateBackend::new(config).await?;
        
        Ok(Self {
            session,
            ctx: BenchmarkContext::new()?,
        })
    }

    pub async fn exec(&mut self, cmd: &str) -> Result<sandbox_types::ExecResult> {
        let handle = self.session.exec(sandbox_types::ExecRequest::new(cmd)).await?;
        let result = handle.wait().await?;
        Ok(result)
    }

    pub async fn snapshot(&mut self) -> Result<sandbox_types::hash::ContentHash> {
        self.session.snapshot().await
    }

    pub async fn fork(&mut self, n: usize) -> Result<Vec<Box<dyn Session>>> {
        let root = self.snapshot().await?;
        let mut forks = Vec::new();
        for _ in 0..n {
            let session = sandbox_session::restore(root).await?;
            forks.push(session);
        }
        Ok(forks)
    }

    pub fn session(&self) -> &dyn Session {
        &*self.session
    }

    pub fn id(&self) -> SessionId {
        self.session.id()
    }
}