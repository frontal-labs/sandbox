use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_runtime::{JsRuntime, RuntimeConfig, RuntimeOptions};
use std::sync::Arc;

pub struct RuntimeBenchHarness {
    pub runtime: JsRuntime,
    pub ctx: BenchmarkContext,
}

impl RuntimeBenchHarness {
    pub async fn new(ctx: &BenchmarkContext) -> Result<Self> {
        let config = RuntimeConfig::default();
        let options = RuntimeOptions::default();
        let runtime = JsRuntime::new(config, options).await?;
        
        Ok(Self {
            runtime,
            ctx: BenchmarkContext::new()?,
        })
    }

    pub async fn execute(&mut self, code: &str) -> Result<serde_json::Value> {
        self.runtime.execute_script("<bench>", code).await
    }

    pub fn runtime(&self) -> &JsRuntime {
        &self.runtime
    }
}

pub const JS_SIMPLE: &str = "1 + 1";
pub const JS_LOOP: &str = "let sum = 0; for (let i = 0; i < 1000; i++) sum += i; sum";
pub const JS_FS_READ: &str = "const fs = require('fs'); fs.readFileSync('/test.txt', 'utf8')";
pub const JS_FS_WRITE: &str = "const fs = require('fs'); fs.writeFileSync('/test.txt', 'hello')";
pub const JS_FETCH: &str = "fetch('https://example.com').then(r => r.text())";