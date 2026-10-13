use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_wasm::{WasmHost, WasmConfig};
use std::sync::Arc;

pub struct WasmBenchHarness {
    pub host: WasmHost,
    pub ctx: BenchmarkContext,
}

impl WasmBenchHarness {
    pub async fn new(ctx: &BenchmarkContext) -> Result<Self> {
        let config = WasmConfig::default();
        let host = WasmHost::new(config).await?;
        
        Ok(Self {
            host,
            ctx: BenchmarkContext::new()?,
        })
    }

    pub async fn load_module(&mut self, name: &str) -> Result<()> {
        self.host.load_module(name).await
    }

    pub async fn execute(&mut self, name: &str, args: &[&str]) -> Result<String> {
        self.host.execute(name, args).await
    }

    pub fn host(&self) -> &WasmHost {
        &self.host
    }
}

pub const WASM_TOOLS: &[&str] = &["jq", "ripgrep", "sqlite3"];