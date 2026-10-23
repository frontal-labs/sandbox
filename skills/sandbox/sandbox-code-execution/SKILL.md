# Frontal Sandbox Code Execution Skill

## Overview
Execute code in sandbox sessions with support for multiple languages, streaming output, and result handling.

## When to Use
- Running code snippets
- Executing scripts
- Running test suites
- Building and compiling projects

## Dependencies
- sandbox-api-client
- sandbox-session-manager
- sandbox-file-operations

## Functions

### `sandbox_exec(session_id, command, args, env, workdir, timeout)`
Execute a command in the sandbox.

### `sandbox_exec_stream(session_id, command, args, env, workdir, timeout)`
Execute with streaming output (stdout/stderr).

### `sandbox_exec_code(session_id, language, code, filename, timeout)`
Execute code directly by language.

### `sandbox_exec_script(session_id, script_path, args, env, timeout)`
Execute a script file.

### `sandbox_build(session_id, build_command, workdir, timeout)`
Run build commands.

### `sandbox_test(session_id, test_command, workdir, timeout)`
Run test commands.

## Supported Languages
- javascript/node: "node"
- python: "python3"
- go: "go run"
- rust: "cargo run"
- java: "java"
- bash: "bash"

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-code-execution/skill.sh"

result=$(sandbox_exec_code "$session_id" "python" 'print("Hello, World!")' "main.py" 30)
echo "$result" | jq -r '.stdout'

# Streaming
sandbox_exec_stream "$session_id" "npm" '["test"]' '{}' "/workspace" 120 | while IFS= read -r line; do
    echo "TEST: $line"
done
```