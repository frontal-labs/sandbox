//! Shell interpreter

use crate::ast::{AstNode, Command};
use crate::builtins::Builtins;
use sandbox_types::SandboxError;
use sandbox_vfs::Vfs;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Shell {
    builtins: Builtins,
    functions: HashMap<String, Vec<AstNode>>,
}

#[derive(Debug, Clone)]
pub struct ShellContext {
    pub env: HashMap<String, String>,
    pub cwd: String,
}

impl Shell {
    pub async fn new(_vfs: Arc<Vfs>, _config: ShellConfig) -> Result<Self, SandboxError> {
        Ok(Self {
            builtins: Builtins::new(),
            functions: HashMap::new(),
        })
    }

    pub async fn execute(&mut self, input: &str) -> Result<ExecutionResult, SandboxError> {
        let ast = parse(input)?;
        let result = self.execute_ast(&ast).await?;
        Ok(result)
    }

    async fn execute_ast(&mut self, ast: &AstNode) -> Result<ExecutionResult, SandboxError> {
        match ast {
            AstNode::Command(cmd) => self.execute_command(cmd).await,
            AstNode::Pipeline(cmds) => self.execute_pipeline(cmds).await,
            AstNode::Sequence(cmds) => self.execute_sequence(cmds).await,
            _ => Ok(ExecutionResult::default()),
        }
    }

    async fn execute_command(&mut self, cmd: &Command) -> Result<ExecutionResult, SandboxError> {
        if let Some(builtin) = self.builtins.get(&cmd.name) {
            let mut args = cmd.args.clone();
            args.insert(0, cmd.name.clone());
            let exit_code = builtin(&args)?;
            return Ok(ExecutionResult { exit_code, stdout: String::new(), stderr: String::new() });
        }

        if self.functions.contains_key(&cmd.name) {
            // Execute function body
            return Ok(ExecutionResult::default());
        }

        Ok(ExecutionResult { exit_code: 127, stdout: String::new(), stderr: format!("command not found: {}", cmd.name) })
    }

    async fn execute_pipeline(&mut self, cmds: &[Command]) -> Result<ExecutionResult, SandboxError> {
        let mut last_result = ExecutionResult::default();
        for cmd in cmds {
            last_result = self.execute_command(cmd).await?;
        }
        Ok(last_result)
    }

    async fn execute_sequence(&mut self, cmds: &[Command]) -> Result<ExecutionResult, SandboxError> {
        let mut last_result = ExecutionResult::default();
        for cmd in cmds {
            last_result = self.execute_command(cmd).await?;
        }
        Ok(last_result)
    }
}

#[derive(Debug, Default)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Default)]
pub struct ShellConfig {}

pub fn parse(input: &str) -> Result<AstNode, SandboxError> {
    let tokens: Vec<&str> = input.trim().split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(AstNode::Command(Command {
            name: String::new(),
            args: Vec::new(),
            redirects: Vec::new(),
            background: false,
        }));
    }

    let name = tokens[0].to_string();
    let args = tokens[1..].iter().map(|s| s.to_string()).collect();

    Ok(AstNode::Command(Command {
        name,
        args,
        redirects: Vec::new(),
        background: false,
    }))
}