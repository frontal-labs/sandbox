//! Shell parser

use crate::ast::{AstNode, Command};
use sandbox_types::SandboxError;

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