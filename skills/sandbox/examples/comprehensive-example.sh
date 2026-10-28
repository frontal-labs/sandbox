#!/usr/bin/env bash
# Comprehensive Example - Using all sandbox skills together

set -euo pipefail

# Load all sandbox skills
source "$(dirname "$0")/../skills.sh"
sandbox_skills_load_all

# Configuration
export SANDBOX_API_URL="${SANDBOX_API_URL:-https://api.sandbox.example.com}"
export SANDBOX_API_KEY="${SANDBOX_API_KEY:-}"

if [[ -z "$SANDBOX_API_KEY" ]]; then
    echo "Error: SANDBOX_API_KEY must be set" >&2
    exit 1
fi

echo "=== Sandbox Comprehensive Example ==="
echo "API URL: $SANDBOX_API_URL"
echo

# 1. Create session from template
echo "1. Creating session from 'node-dev' template..."
session=$(sandbox_lifecycle_create_from_template "node-dev" '{"timeout": 7200}')
session_id=$(echo "$session" | jq -r '.id')
echo "   Session ID: $session_id"

# 2. Wait for session to be ready
echo "2. Waiting for session to be ready..."
session=$(sandbox_session_wait_ready "$session_id" 120)
status=$(echo "$session" | jq -r '.status')
echo "   Status: $status"

# 3. Write application files
echo "3. Writing application files..."
sandbox_file_write "$session_id" "/workspace/package.json" '{
  "name": "sandbox-demo",
  "version": "1.0.0",
  "scripts": {
    "dev": "node server.js",
    "test": "echo \"Tests passed\""
  },
  "dependencies": {
    "express": "^4.18.0"
  }
}' "0644"

sandbox_file_write "$session_id" "/workspace/server.js" 'const express = require("express");
const app = express();
const PORT = process.env.PORT || 3000;

app.get("/", (req, res) => {
  res.json({ message: "Hello from Sandbox!", timestamp: new Date().toISOString() });
});

app.get("/health", (req, res) => {
  res.json({ status: "ok" });
});

app.listen(PORT, "0.0.0.0", () => {
  console.log(`Server running on port ${PORT}`);
});' "0644"

# 4. Install dependencies
echo "4. Installing dependencies..."
sandbox_install_deps "$session_id" "npm" "/workspace" 120

# 5. Start development server in background
echo "5. Starting development server..."
server_result=$(sandbox_exec "$session_id" "npm" '["run", "dev"]' '{}' "/workspace" 10)
echo "   Server started: $(echo "$server_result" | jq -r '.stdout // "running"')"

# Wait a moment for server to start
sleep 3

# 6. Create preview URL
echo "6. Creating preview URL..."
preview=$(sandbox_preview_create "$session_id" 3000 '{"expiry": 3600}')
preview_url=$(echo "$preview" | jq -r '.url')
echo "   Preview URL: $preview_url"

# 7. Test the preview
echo "7. Testing preview endpoint..."
sleep 2
test_result=$(sandbox_exec "$session_id" "curl" '["-s", "http://localhost:3000/health"]' '{}' "/workspace" 10)
echo "   Health check: $(echo "$test_result" | jq -r '.stdout')"

# 8. Create a terminal for interactive debugging
echo "8. Creating terminal session..."
terminal=$(sandbox_terminal_create "$session_id" 120 40 "bash")
terminal_id=$(echo "$terminal" | jq -r '.id')
ws_url=$(sandbox_terminal_connect "$session_id" "$terminal_id")
echo "   Terminal WebSocket: $ws_url"

# 9. Run tests
echo "9. Running tests..."
test_result=$(sandbox_test "$session_id" "npm test" "/workspace" 60)
echo "   Test result: $(echo "$test_result" | jq -r '.stdout')"

# 10. Snapshot the session
echo "10. Creating snapshot..."
snapshot=$(sandbox_lifecycle_snapshot "$session_id" "after-tests" "State after tests passed")
snapshot_id=$(echo "$snapshot" | jq -r '.id')
echo "   Snapshot ID: $snapshot_id"

# 11. List files
echo "11. Listing workspace files..."
files=$(sandbox_file_list "$session_id" "/workspace" true)
echo "$files" | jq '.'

# 12. Cleanup
echo "12. Cleaning up..."
sandbox_terminal_close "$session_id" "$terminal_id"
sandbox_preview_delete "$session_id" "$(echo "$preview" | jq -r '.id')"
sandbox_session_delete "$session_id"
echo "   Cleanup complete"

echo
echo "=== Example completed successfully ==="