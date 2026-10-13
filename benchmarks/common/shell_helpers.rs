use crate::BenchmarkContext;
use anyhow::Result;
use sandbox_shell::{Shell, ShellConfig};
use sandbox_vfs::Vfs;
use std::sync::Arc;

pub struct ShellBenchHarness {
    pub shell: Shell,
    pub ctx: BenchmarkContext,
}

impl ShellBenchHarness {
    pub async fn new(ctx: &BenchmarkContext) -> Result<Self> {
        let vfs = Arc::new(Vfs::new(sandbox_vfs::VfsConfig::default()).await?);
        let config = ShellConfig::default();
        let shell = Shell::new(vfs, config).await?;
        
        Ok(Self {
            shell,
            ctx: BenchmarkContext::new()?,
        })
    }

    pub async fn exec(&mut self, cmd: &str) -> Result<String> {
        let output = self.shell.execute(cmd).await?;
        Ok(output.stdout)
    }

    pub fn shell(&self) -> &Shell {
        &self.shell
    }
}

pub const SIMPLE_COMMANDS: &[&str] = &[
    "echo hello",
    "true",
    "false",
    "pwd",
    "cd /",
];

pub const PIPE_COMMANDS: &[&str] = &[
    "echo hello | cat",
    "echo -e 'a\nb\nc' | grep b",
    "echo hello | tr a-z A-Z",
    "seq 1 100 | head -5",
];

pub const REDIRECT_COMMANDS: &[&str] = &[
    "echo hello > /tmp/out.txt",
    "echo hello >> /tmp/out.txt",
    "cat < /tmp/out.txt",
    "echo hello 2>&1",
];

pub const COMPLEX_COMMANDS: &[&str] = &[
    "for i in {1..10}; do echo $i; done",
    "if true; then echo yes; else echo no; fi",
    "func() { echo $1; }; func hello",
    "export VAR=value && echo $VAR",
];