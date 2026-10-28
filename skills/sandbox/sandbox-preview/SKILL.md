# Frontal Sandbox Preview Skill

## Overview
Generate and manage preview URLs for sandbox sessions. Handles automatic port detection, HTTPS, authentication, and custom domains.

## When to Use
- Sharing sandbox previews with team
- Preview deployments for PRs
- Temporary public access to sandbox apps
- Integration with CI/CD preview workflows

## Dependencies
- sandbox-api-client
- sandbox-session-manager
- sandbox-port-tunneling

## Functions

### `sandbox_preview_create(session_id, port, options)`
Create a preview URL for a port.

### `sandbox_preview_list(session_id)`
List all preview URLs for a session.

### `sandbox_preview_get(session_id, preview_id)`
Get preview details.

### `sandbox_preview_delete(session_id, preview_id)`
Delete a preview URL.

### `sandbox_preview_update(session_id, preview_id, updates)`
Update preview settings (password, expiry, etc.).

### `sandbox_auto_preview(session_id, command, port)`
Auto-detect port and create preview for a command.

## Options
- `password`: Optional password protection
- `expiry`: Expiration time in seconds (default: 3600)
- `custom_domain`: Custom subdomain
- `path`: Path prefix

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-preview/skill.sh"

# Create preview for port 3000
preview=$(sandbox_preview_create "$session_id" 3000 '{"expiry": 7200, "password": "secret123"}')
url=$(echo "$preview" | jq -r '.url')
echo "Preview: $url (password: secret123)"

# Auto preview for dev server
sandbox_auto_preview "$session_id" "npm run dev" 3000
```