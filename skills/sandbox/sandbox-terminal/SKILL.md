# Frontal Sandbox Terminal Skill

## Overview
Interactive terminal/PTY management for sandbox sessions. Supports websocket-based terminal sessions with resize, input/output streaming.

## When to Use
- Interactive terminal sessions
- Running interactive commands (vim, REPLs, etc.)
- Debugging with live terminal access
- Building terminal-based UIs

## Dependencies
- sandbox-api-client
- sandbox-session-manager

## Functions

### `sandbox_terminal_create(session_id, cols, rows, shell)`
Create a new terminal session.

### `sandbox_terminal_connect(session_id, terminal_id)`
Get websocket connection URL for terminal.

### `sandbox_terminal_write(session_id, terminal_id, data)`
Send input to terminal.

### `sandbox_terminal_resize(session_id, terminal_id, cols, rows)`
Resize terminal.

### `sandbox_terminal_close(session_id, terminal_id)`
Close terminal session.

### `sandbox_terminal_list(session_id)`
List active terminals for a session.

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-terminal/skill.sh"

terminal=$(sandbox_terminal_create "$session_id" 80 24 "bash")
terminal_id=$(echo "$terminal" | jq -r '.id')
ws_url=$(sandbox_terminal_connect "$session_id" "$terminal_id")

# Connect via websocket client
# wscat -c "$ws_url"
```