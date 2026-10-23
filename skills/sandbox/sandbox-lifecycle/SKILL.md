# Frontal Sandbox Lifecycle Skill

## Overview
High-level lifecycle management for sandbox sessions including templates, snapshots, and automated workflows.

## When to Use
- Creating reproducible environments
- Snapshotting and restoring state
- Template-based session creation
- Automated cleanup policies

## Dependencies
- sandbox-api-client
- sandbox-session-manager
- sandbox-file-operations

## Functions

### `sandbox_lifecycle_create_from_template(template_name, overrides)`
Create session from a predefined template.

### `sandbox_lifecycle_snapshot(session_id, name, description)`
Create a snapshot of session state.

### `sandbox_lifecycle_restore(snapshot_id, new_session_options)`
Restore session from snapshot.

### `sandbox_lifecycle_list_templates()`
List available templates.

### `sandbox_lifecycle_list_snapshots(session_id)`
List snapshots for a session.

### `sandbox_lifecycle_delete_snapshot(snapshot_id)`
Delete a snapshot.

### `sandbox_lifecycle_set_ttl(session_id, ttl_seconds)`
Set time-to-live for auto-cleanup.

### `sandbox_lifecycle_extend_ttl(session_id, additional_seconds)`
Extend session TTL.

### `sandbox_lifecycle_get_events(session_id)`
Get session lifecycle events.

## Templates
Templates define base images, dependencies, and configurations:
- node-dev: Node.js development
- python-dev: Python development
- go-dev: Go development
- rust-dev: Rust development
- fullstack: Full stack with Node + PostgreSQL
- ml: Machine learning with Python + CUDA

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-lifecycle/skill.sh"

# Create from template
session=$(sandbox_lifecycle_create_from_template "node-dev" '{"timeout": 7200}')

# Snapshot current state
snapshot=$(sandbox_lifecycle_snapshot "$session_id" "before-deploy" "State before deployment")

# Restore later
new_session=$(sandbox_lifecycle_restore "$snapshot_id" '{"timeout": 3600}')
```