# Frontal Sandbox Skills

Comprehensive AI agent skills for interacting with and building on the Frontal Sandbox project API.

## Skills Overview

| Skill | Description | Dependencies |
|-------|-------------|--------------|
| `sandbox-api-client` | Core HTTP client with auth, retries, error handling | - |
| `sandbox-session-manager` | Session lifecycle: create, list, get, update, delete | api-client |
| `sandbox-file-operations` | Read, write, list, upload, download, sync files | api-client, session-manager |
| `sandbox-code-execution` | Execute code, scripts, builds, tests in sessions | api-client, session-manager, file-ops |
| `sandbox-terminal` | Interactive PTY/terminal with websocket support | api-client, session-manager |
| `sandbox-port-tunneling` | Port forwarding, public URLs, custom protocols | api-client, session-manager |
| `sandbox-preview` | Preview URLs with auth, expiry, custom domains | api-client, session-manager, tunneling |
| `sandbox-lifecycle` | Templates, snapshots, TTL, lifecycle events | api-client, session-manager, file-ops |
| `sandbox-ai-runner` | AI/ML inference, agents, training, deployment | api-client, session-manager, file-ops, code-exec, tunneling |

## Quick Start

```bash
# Set environment variables
export SANDBOX_API_URL="https://api.sandbox.example.com"
export SANDBOX_API_KEY="your-api-key"

# Load all skills
source "/path/to/sandbox/skills.sh"
sandbox_skills_load_all

# Create a session
session=$(sandbox_session_create "node:20" 300)
session_id=$(echo "$session" | jq -r '.id')

# Write and execute code
sandbox_file_write "$session_id" "/workspace/main.js" 'console.log("Hello")'
result=$(sandbox_exec_code "$session_id" "javascript" 'console.log("Hello")')

# Create preview URL
preview=$(sandbox_preview_create "$session_id" 3000)
url=$(echo "$preview" | jq -r '.url')
```

## Installation

The skills are designed to be sourced directly:

```bash
# In your script or shell
source "/path/to/sandbox/skills/skills.sh"
sandbox_skills_load_all
```

## Configuration

Required environment variables:
- `SANDBOX_API_URL` - Base URL for Frontal Sandbox API
- `SANDBOX_API_KEY` - Authentication token

Optional:
- `SANDBOX_SKILLS_DIR` - Override skills directory path
- `SANDBOX_MAX_RETRIES` - Max retry attempts (default: 3)
- `SANDBOX_RETRY_DELAY` - Initial retry delay in seconds (default: 1)

## Examples

See `examples/comprehensive-example.sh` for a full workflow demonstrating all skills.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     sandbox-ai-runner                       │
│  (inference, agents, training, evaluation, deployment)     │
└─────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
┌───────────────┐    ┌───────────────┐    ┌───────────────┐
│ sandbox-code- │    │ sandbox-term- │    │ sandbox-port- │
│ execution     │    │ inal          │    │ tunneling     │
└───────────────┘    └───────────────┘    └───────────────┘
        │                     │                     │
        └─────────────────────┼─────────────────────┘
                              ▼
                   ┌───────────────────────┐
                   │ sandbox-session-mgr   │
                   │ (create, list, wait)  │
                   └───────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
┌───────────────┐    ┌───────────────┐    ┌───────────────┐
│ sandbox-file- │    │ sandbox-pre-  │    │ sandbox-life- │
│ operations    │    │ view          │    │ cycle         │
└───────────────┘    └───────────────┘    └───────────────┘
        │                     │                     │
        └─────────────────────┼─────────────────────┘
                              ▼
                   ┌───────────────────────┐
                   │ sandbox-api-client    │
                   │ (HTTP, auth, retry)   │
                   └───────────────────────┘
```

## Error Handling

All skills follow consistent error handling:
- Functions return 0 on success, non-zero on failure
- Errors logged to stderr with timestamps
- JSON error responses include status codes
- Automatic retry on rate limits (429) with exponential backoff

## Testing

```bash
# Run the comprehensive example
./examples/comprehensive-example.sh

# Test individual skills
source skills.sh
sandbox_skills_load_all
sandbox_session_create "alpine:latest" 60
```

## Extending Skills

To add a new skill:
1. Create directory under `sandbox/` with skill name
2. Add `SKILL.md` documentation
3. Add `skill.sh` with functions
4. Add to `skills.sh` skills array
5. Follow naming convention: `sandbox_<skill>_<function>`

## License

Apache-2.0