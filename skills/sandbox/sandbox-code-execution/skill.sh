#!/usr/bin/env bash
# Frontal Sandbox Code Execution - Execute code in sandbox sessions

set -euo pipefail

SANDBOX_CODE_EXEC_VERSION="1.0.0"

declare -A SANDBOX_LANG_COMMANDS=(
    ["javascript"]="node"
    ["js"]="node"
    ["typescript"]="npx ts-node"
    ["ts"]="npx ts-node"
    ["python"]="python3"
    ["py"]="python3"
    ["go"]="go run"
    ["rust"]="cargo run --quiet"
    ["java"]="java"
    ["bash"]="bash"
    ["sh"]="bash"
    ["zsh"]="zsh"
)

sandbox_exec() {
    local session_id="$1"
    local command="$2"
    local args="${3:-[]}"
    local env="${4:-{}}"
    local workdir="${5:-/workspace}"
    local timeout="${6:-60}"

    local payload
    payload=$(jq -n \
        --arg cmd "$command" \
        --argjson args "$args" \
        --argjson env "$env" \
        --arg cwd "$workdir" \
        --argjson timeout "$timeout" \
        '{command: $cmd, args: $args, env: $env, cwd: $cwd, timeout: $timeout}')

    sandbox_api_post "/v1/sessions/$session_id/exec" "$payload"
}

sandbox_exec_stream() {
    local session_id="$1"
    local command="$2"
    local args="${3:-[]}"
    local env="${4:-{}}"
    local workdir="${5:-/workspace}"
    local timeout="${6:-60}"

    local payload
    payload=$(jq -n \
        --arg cmd "$command" \
        --argjson args "$args" \
        --argjson env "$env" \
        --arg cwd "$workdir" \
        --argjson timeout "$timeout" \
        --argjson stream true \
        '{command: $cmd, args: $args, env: $env, cwd: $cwd, timeout: $timeout, stream: $stream}')

    local url="${SANDBOX_API_URL%/}/v1/sessions/$session_id/exec"
    curl -sS -N -X POST \
        -H "Authorization: Bearer $SANDBOX_API_KEY" \
        -H "Content-Type: application/json" \
        -H "Accept: text/event-stream" \
        --data "$payload" \
        "$url" | while IFS= read -r line; do
        if [[ "$line" =~ ^data:\ (.*)$ ]]; then
            echo "${BASH_REMATCH[1]}"
        fi
    done
}

sandbox_exec_code() {
    local session_id="$1"
    local language="$2"
    local code="$3"
    local filename="${4:-main}"
    local timeout="${5:-30}"

    local cmd="${SANDBOX_LANG_COMMANDS[$language]:-$language}"
    local ext=""

    case "$language" in
        javascript|js) ext=".js" ;;
        typescript|ts) ext=".ts" ;;
        python|py) ext=".py" ;;
        go) ext=".go" ;;
        rust) ext=".rs" ;;
        java) ext=".java" ;;
        bash|sh) ext=".sh" ;;
        *) ext="" ;;
    esac

    local filepath="/workspace/${filename}${ext}"
    sandbox_file_write "$session_id" "$filepath" "$code" "0644"

    if [[ "$language" == "rust" ]]; then
        sandbox_file_write "$session_id" "/workspace/Cargo.toml" '[package]
name = "sandbox"
version = "0.1.0"
edition = "2021"

[dependencies]' "0644"
        sandbox_file_write "$session_id" "/workspace/src/main.rs" "$code" "0644"
        filepath="/workspace"
        cmd="cargo run --quiet"
    fi

    sandbox_exec "$session_id" "$cmd" "[$(jq -n --arg f "$filepath" '$f')]" '{}' "/workspace" "$timeout"
}

sandbox_exec_script() {
    local session_id="$1"
    local script_path="$2"
    local args="${3:-[]}"
    local env="${4:-{}}"
    local timeout="${5:-60}"

    sandbox_exec "$session_id" "bash" "[$(jq -n --arg s "$script_path" '$s')] + $args" "$env" "/workspace" "$timeout"
}

sandbox_build() {
    local session_id="$1"
    local build_command="$2"
    local workdir="${3:-/workspace}"
    local timeout="${4:-120}"

    sandbox_exec "$session_id" "bash" "[$(jq -n --arg c "$build_command" '-c')]" '{}' "$workdir" "$timeout"
}

sandbox_test() {
    local session_id="$1"
    local test_command="$2"
    local workdir="${3:-/workspace}"
    local timeout="${4:-180}"

    sandbox_exec "$session_id" "bash" "[$(jq -n --arg c "$test_command" '-c')]" '{}' "$workdir" "$timeout"
}

sandbox_install_deps() {
    local session_id="$1"
    local package_manager="${2:-npm}"
    local workdir="${3:-/workspace}"
    local timeout="${4:-120}"

    local cmd=""
    case "$package_manager" in
        npm) cmd="npm install" ;;
        yarn) cmd="yarn install" ;;
        pnpm) cmd="pnpm install" ;;
        pip) cmd="pip install -r requirements.txt" ;;
        poetry) cmd="poetry install" ;;
        cargo) cmd="cargo build" ;;
        go) cmd="go mod download" ;;
        *) cmd="$package_manager install" ;;
    esac

    sandbox_build "$session_id" "$cmd" "$workdir" "$timeout"
}