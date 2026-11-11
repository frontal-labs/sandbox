//! `frontal-sandbox mcp`: a Model Context Protocol server over stdio (PLAN §6 M6). It
//! gives an agent four tools, `exec`, `read_file`, `write_file`, and `list_dir`, backed by
//! one in-process sandbox session. Each message is one JSON object per line, as MCP's
//! stdio transport defines it. The session lives as long as the process does.

use sandbox_session::{IsolateBackend, Session};
use sandbox_types::{SandboxError, SessionConfig};
use sandbox_vfs::OpenFlags;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;

/// The protocol revision this server speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The tools a client sees, with their argument schemas.
fn tool_list() -> Value {
    json!({
        "tools": [
            {
                "name": "exec",
                "description": "Run a shell command in the sandbox. Working directory and environment persist between calls.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"]
                }
            },
            {
                "name": "read_file",
                "description": "Read a file in the sandbox as UTF-8 text.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            },
            {
                "name": "write_file",
                "description": "Create or replace a file in the sandbox.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            },
            {
                "name": "list_dir",
                "description": "List the entries of a directory in the sandbox, one per line.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        ]
    })
}

/// A JSON-RPC error response for `id`.
fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool result: text content, and `isError` when the tool failed.
fn tool_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument '{name}'"))
}

/// Reads a whole file from the session's filesystem.
async fn read_whole(session: &IsolateBackend, path: &str) -> Result<Vec<u8>, SandboxError> {
    let fs = session.fs();
    let stat = fs
        .stat(Path::new(path))
        .await
        .map_err(sandbox_session::vfs_error)?;
    let flags = OpenFlags::default();
    let handle = fs
        .open(Path::new(path), flags)
        .await
        .map_err(sandbox_session::vfs_error)?;
    let bytes = fs
        .read(&handle, 0, stat.size as usize)
        .await
        .map_err(sandbox_session::vfs_error);
    fs.close(&handle);
    bytes.map(|b| b.to_vec())
}

/// Runs one tool call and returns its MCP result.
async fn call_tool(session: &IsolateBackend, name: &str, args: &Value) -> Value {
    let outcome: Result<String, String> = match name {
        "exec" => match string_arg(args, "command") {
            Err(e) => Err(e),
            Ok(command) => match session.run_script(command).await {
                Ok(out) => {
                    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    if !stderr.is_empty() {
                        text.push_str(&format!("\n[stderr]\n{stderr}"));
                    }
                    text.push_str(&format!("\n[exit {}]", out.exit_code));
                    Ok(text)
                }
                Err(e) => Err(e.message),
            },
        },
        "read_file" => match string_arg(args, "path") {
            Err(e) => Err(e),
            Ok(path) => read_whole(session, path)
                .await
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .map_err(|e| e.message),
        },
        "write_file" => match (string_arg(args, "path"), string_arg(args, "content")) {
            (Ok(path), Ok(content)) => {
                sandbox_session::write_file(session, path, content.as_bytes())
                    .await
                    .map(|()| format!("wrote {} bytes to {path}", content.len()))
                    .map_err(|e| e.message)
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "list_dir" => match string_arg(args, "path") {
            Err(e) => Err(e),
            Ok(path) => match session.fs().readdir(Path::new(path)).await {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .into_iter()
                        .map(|e| {
                            if e.is_dir {
                                format!("{}/", e.name)
                            } else {
                                e.name
                            }
                        })
                        .collect();
                    names.sort();
                    Ok(names.join("\n"))
                }
                Err(e) => Err(sandbox_session::vfs_error(e).message),
            },
        },
        other => return tool_result(format!("unknown tool '{other}'"), true),
    };
    match outcome {
        Ok(text) => tool_result(text, false),
        Err(message) => tool_result(message, true),
    }
}

/// Handles one JSON-RPC message. Returns the response, or `None` for a notification,
/// which takes no response.
pub async fn handle(session: &IsolateBackend, message: &Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(rpc_error(
            Value::Null,
            INVALID_REQUEST,
            "a message must be a JSON object",
        ));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "missing method",
        ));
    };
    // A message without an id is a notification, and gets no reply.
    let id = id?;
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "frontal-sandbox", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => tool_list(),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(
                    id,
                    INVALID_PARAMS,
                    "tools/call needs a tool name",
                ));
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            call_tool(session, name, &args).await
        }
        other => {
            return Some(rpc_error(
                id,
                METHOD_NOT_FOUND,
                &format!("unknown method '{other}'"),
            ))
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Serves MCP on stdin and stdout until stdin closes.
pub fn serve_stdio() -> Result<i32, SandboxError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SandboxError::internal(format!("failed to start runtime: {e}")))?;
    runtime.block_on(async {
        let session = IsolateBackend::new(SessionConfig::default()).await?;
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        for line in stdin.lock().lines() {
            let line =
                line.map_err(|e| SandboxError::internal(format!("failed to read stdin: {e}")))?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(message) => handle(&session, &message).await,
                Err(e) => Some(rpc_error(
                    Value::Null,
                    PARSE_ERROR,
                    &format!("parse error: {e}"),
                )),
            };
            if let Some(reply) = reply {
                writeln!(stdout, "{reply}")
                    .and_then(|()| stdout.flush())
                    .map_err(|e| SandboxError::internal(format!("failed to write stdout: {e}")))?;
            }
        }
        Ok(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn initialize_reports_the_protocol_and_the_tools_capability() {
        let rt = session();
        rt.block_on(async {
            let backend = IsolateBackend::new(SessionConfig::default()).await.unwrap();
            let reply = handle(
                &backend,
                &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            )
            .await
            .unwrap();
            assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
            assert!(reply["result"]["capabilities"]["tools"].is_object());
        });
    }

    #[test]
    fn notifications_get_no_reply() {
        let rt = session();
        rt.block_on(async {
            let backend = IsolateBackend::new(SessionConfig::default()).await.unwrap();
            let reply = handle(
                &backend,
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            )
            .await;
            assert!(reply.is_none());
        });
    }

    #[test]
    fn tools_list_names_the_four_tools() {
        let names: Vec<String> = tool_list()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["exec", "read_file", "write_file", "list_dir"]);
    }

    #[test]
    fn exec_runs_in_the_session_and_keeps_its_state() {
        let rt = session();
        rt.block_on(async {
            let backend = IsolateBackend::new(SessionConfig::default()).await.unwrap();
            let call = |command: &str| {
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                       "params": {"name": "exec", "arguments": {"command": command}}})
            };
            let first = handle(&backend, &call("export KEPT=yes; echo hi"))
                .await
                .unwrap();
            assert_eq!(first["result"]["isError"], false, "{first}");
            assert!(first["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("hi"));
            let second = handle(&backend, &call("echo $KEPT")).await.unwrap();
            assert!(second["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("yes"));
        });
    }

    #[test]
    fn write_then_read_and_list_go_through_the_filesystem() {
        let rt = session();
        rt.block_on(async {
            let backend = IsolateBackend::new(SessionConfig::default()).await.unwrap();
            let tool = |name: &str, arguments: Value| {
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                       "params": {"name": name, "arguments": arguments}})
            };
            let wrote = handle(
                &backend,
                &tool(
                    "write_file",
                    json!({"path": "/notes.txt", "content": "hello\n"}),
                ),
            )
            .await
            .unwrap();
            assert_eq!(wrote["result"]["isError"], false, "{wrote}");
            let read = handle(&backend, &tool("read_file", json!({"path": "/notes.txt"})))
                .await
                .unwrap();
            assert_eq!(read["result"]["content"][0]["text"], "hello\n");
            let listed = handle(&backend, &tool("list_dir", json!({"path": "/"})))
                .await
                .unwrap();
            assert!(listed["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("notes.txt"));
        });
    }

    #[test]
    fn a_missing_argument_is_a_tool_error_not_a_protocol_error() {
        let rt = session();
        rt.block_on(async {
            let backend = IsolateBackend::new(SessionConfig::default()).await.unwrap();
            let reply = handle(
                &backend,
                &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                        "params": {"name": "exec", "arguments": {}}}),
            )
            .await
            .unwrap();
            assert!(reply.get("error").is_none());
            assert_eq!(reply["result"]["isError"], true);
        });
    }

    #[test]
    fn an_unknown_method_is_a_protocol_error() {
        let rt = session();
        rt.block_on(async {
            let backend = IsolateBackend::new(SessionConfig::default()).await.unwrap();
            let reply = handle(
                &backend,
                &json!({"jsonrpc": "2.0", "id": 5, "method": "nope"}),
            )
            .await
            .unwrap();
            assert_eq!(reply["error"]["code"], METHOD_NOT_FOUND);
        });
    }
}
