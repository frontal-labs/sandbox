# Frontal Sandbox File Operations Skill

## Overview
File system operations within sandbox sessions: read, write, list, upload, download, and sync.

## When to Use
- Managing files in sandbox sessions
- Uploading project files for execution
- Reading output files
- Synchronizing local directories

## Dependencies
- sandbox-api-client
- sandbox-session-manager

## Functions

### `sandbox_file_read(session_id, path)`
Read a file from the sandbox.

### `sandbox_file_write(session_id, path, content, mode)`
Write content to a file in the sandbox.

### `sandbox_file_list(session_id, path, recursive)`
List directory contents.

### `sandbox_file_upload(session_id, local_path, remote_path)`
Upload a local file to the sandbox.

### `sandbox_file_download(session_id, remote_path, local_path)`
Download a file from the sandbox.

### `sandbox_file_delete(session_id, path, recursive)`
Delete a file or directory.

### `sandbox_file_mkdir(session_id, path, recursive)`
Create directory.

### `sandbox_file_exists(session_id, path)`
Check if file/directory exists.

### `sandbox_file_stat(session_id, path)`
Get file metadata.

### `sandbox_dir_sync(session_id, local_dir, remote_dir, exclude_patterns)`
Sync local directory to sandbox.

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-file-operations/skill.sh"

sandbox_file_write "$session_id" "/workspace/main.js" 'console.log("Hello")' "0644"
sandbox_file_upload "$session_id" "./package.json" "/workspace/package.json"
content=$(sandbox_file_read "$session_id" "/workspace/output.txt")
```