#!/usr/bin/env bash
# Frontal Sandbox Skills Loader - Loads all sandbox skills

set -euo pipefail

SANDBOX_SKILLS_DIR="${SANDBOX_SKILLS_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
SANDBOX_SKILLS_VERSION="1.0.0"

# Colors for output
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly NC='\033[0m'

_sandbox_skills_log() {
    local level="$1"
    shift
    local color="$NC"
    case "$level" in
        INFO) color="$GREEN" ;;
        WARN) color="$YELLOW" ;;
        ERROR) color="$RED" ;;
        DEBUG) color="$BLUE" ;;
    esac
    echo -e "${color}[$(date -u +%Y-%m-%dT%H:%M:%SZ)] [$level] sandbox-skills: $*${NC}" >&2
}

_sandbox_skills_load() {
    local skill_name="$1"
    local skill_path="$SANDBOX_SKILLS_DIR/$skill_name/skill.sh"

    if [[ ! -f "$skill_path" ]]; then
        _sandbox_skills_log "ERROR" "Skill not found: $skill_name at $skill_path"
        return 1
    fi

    # shellcheck source=/dev/null
    source "$skill_path"
    _sandbox_skills_log "DEBUG" "Loaded skill: $skill_name"
}

sandbox_skills_load_all() {
    local skills=(
        "sandbox-api-client"
        "sandbox-session-manager"
        "sandbox-file-operations"
        "sandbox-code-execution"
        "sandbox-terminal"
        "sandbox-port-tunneling"
        "sandbox-preview"
        "sandbox-lifecycle"
        "sandbox-ai-runner"
    )

    _sandbox_skills_log "INFO" "Loading sandbox skills v$SANDBOX_SKILLS_VERSION"

    for skill in "${skills[@]}"; do
        _sandbox_skills_load "$skill"
    done

    _sandbox_skills_log "INFO" "All sandbox skills loaded successfully"
}

sandbox_skills_list() {
    echo "Available sandbox skills:"
    local skills=(
        "sandbox-api-client:Core HTTP client for Frontal Sandbox API"
        "sandbox-session-manager:Session lifecycle management"
        "sandbox-file-operations:File system operations in sandbox"
        "sandbox-code-execution:Code execution in sandbox sessions"
        "sandbox-terminal:Interactive terminal/PTY management"
        "sandbox-port-tunneling:Port forwarding and public URLs"
        "sandbox-preview:Preview URL management"
        "sandbox-lifecycle:Templates, snapshots, lifecycle management"
        "sandbox-ai-runner:AI/ML workload execution"
    )

    for skill_desc in "${skills[@]}"; do
        local skill="${skill_desc%%:*}"
        local desc="${skill_desc#*:}"
        echo "  - $skill: $desc"
    done
}

sandbox_skills_help() {
    cat <<EOF
Frontal Sandbox Skills v$SANDBOX_SKILLS_VERSION

Usage:
  source /path/to/sandbox-skills.sh
  sandbox_skills_load_all

Environment Variables:
  SANDBOX_API_URL     - Frontal Sandbox API base URL (required)
  SANDBOX_API_KEY     - API authentication key (required)
  SANDBOX_SKILLS_DIR  - Skills directory (auto-detected)

Commands:
  sandbox_skills_load_all  - Load all skills
  sandbox_skills_list      - List available skills
  sandbox_skills_help      - Show this help

Skills are organized by domain:
  Core:        sandbox-api-client
  Sessions:    sandbox-session-manager, sandbox-lifecycle
  Files:       sandbox-file-operations
  Execution:   sandbox-code-execution, sandbox-terminal
  Networking:  sandbox-port-tunneling, sandbox-preview
  AI:          sandbox-ai-runner

Example:
  export SANDBOX_API_URL="https://api.sandbox.example.com"
  export SANDBOX_API_KEY="sk-..."
  source "\$SANDBOX_SKILLS_DIR/skills.sh"
  sandbox_skills_load_all

  session=\$(sandbox_session_create "node:20" 300)
  session_id=\$(echo "\$session" | jq -r '.id')
  sandbox_file_write "\$session_id" "/workspace/app.js" 'console.log("Hello")'
  sandbox_exec_code "\$session_id" "javascript" 'console.log("Hello")'
  preview=\$(sandbox_preview_create "\$session_id" 3000)
EOF
}

# Auto-load if sourced directly (not via sandbox_skills_load_all)
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    sandbox_skills_help
fi