use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_promote::{PromotionManager, PromotionConfig};
use sandbox_session::{IsolateBackend, Session, SessionConfig, Tier};
use std::sync::Arc;

pub struct PromotionBenchHarness {
    pub promotion_mgr: PromotionManager,
    pub session: Box<dyn Session>,
    pub ctx: BenchmarkContext,
}

impl PromotionBenchHarness {
    pub async fn new(ctx: &BenchmarkContext) -> Result<Self> {
        let config = SessionConfig::default().with_tier(Tier::Isolate);
        let session = IsolateBackend::new(config).await?;
        
        let promo_config = PromotionConfig::default();
        let promotion_mgr = PromotionManager::new(promo_config).await?;
        
        Ok(Self {
            promotion_mgr,
            session,
            ctx: BenchmarkContext::new()?,
        })
    }

    pub async fn promote(&mut self) -> Result<Box<dyn Session>> {
        let vfs_root = self.session.fs().commit().await?;
        let promoted = self.promotion_mgr.promote(self.session.id(), vfs_root).await?;
        Ok(promoted)
    }

    pub fn promotion_mgr(&self) -> &PromotionManager {
        &self.promotion_mgr
    }

    pub fn session(&self) -> &dyn Session {
        &*self.session
    }
}