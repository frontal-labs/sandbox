# Frontal Sandbox Session Manager Skill

## Overview
Manages sandbox sessions lifecycle: create, list, get, update, and delete sessions.

## When to Use
- Creating new sandbox environments
- Managing existing sessions
- Session pooling and reuse

## Dependencies
- sandbox-api-client

## Functions

### `sandbox_session_create(image, timeout, env_vars, resources)`
Create a new sandbox session.

**Parameters:**
- `image`: Docker image to use (e.g., "node:20", "python:3.11")
- `timeout`: Session timeout in seconds (default: 300)
- `env_vars`: JSON object of environment variables
- `resources`: JSON object with cpu, memory limits

**Returns:** Session object with id, status, connection info

### `sandbox_session_get(session_id)`
Get session details by ID.

### `sandbox_session_list(status, limit, offset)`
List sessions with optional filtering.

### `sandbox_session_update(session_id, updates)`
Update session properties (timeout, env_vars).

### `sandbox_session_delete(session_id)`
Delete/terminate a session.

### `sandbox_session_wait_ready(session_id, timeout)`
Wait for session to reach ready state.

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-session-manager/skill.sh"
source "$SANDBOX_SKILLS_DIR/sandbox-api-client/skill.sh"

session=$(sandbox_session_create "node:20" 600 '{"NODE_ENV": "development"}' '{"cpu": 2, "memory": "4g"}')
session_id=$(echo "$session" | jq -r '.id')
sandbox_session_wait_ready "$session_id" 120
```