#!/usr/bin/env bash
# Frontal Sandbox Terminal - PTY/Terminal management

set -euo pipefail

SANDBOX_TERMINAL_VERSION="1.0.0"

sandbox_terminal_create() {
    local session_id="$1"
    local cols="${2:-80}"
    local rows="${3:-24}"
    local shell="${4:-bash}"

    local payload
    payload=$(jq -n \
        --argjson cols "$cols" \
        --argjson rows "$rows" \
        --arg shell "$shell" \
        '{cols: $cols, rows: $rows, shell: $shell}')

    sandbox_api_post "/v1/sessions/$session_id/terminals" "$payload"
}

sandbox_terminal_connect() {
    local session_id="$1"
    local terminal_id="$2"

    local response
    response=$(sandbox_api_get "/v1/sessions/$session_id/terminals/$terminal_id/connect")
    echo "$response" | jq -r '.url // empty'
}

sandbox_terminal_write() {
    local session_id="$1"
    local terminal_id="$2"
    local data="$3"

    local payload
    payload=$(jq -n --arg data "$data" '{data: $data}')
    sandbox_api_post "/v1/sessions/$session_id/terminals/$terminal_id/write" "$payload"
}

sandbox_terminal_resize() {
    local session_id="$1"
    local terminal_id="$2"
    local cols="$3"
    local rows="$4"

    local payload
    payload=$(jq -n --argjson cols "$cols" --argjson rows "$rows" '{cols: $cols, rows: $rows}')
    sandbox_api_post "/v1/sessions/$session_id/terminals/$terminal_id/resize" "$payload"
}

sandbox_terminal_close() {
    local session_id="$1"
    local terminal_id="$2"
    sandbox_api_delete "/v1/sessions/$session_id/terminals/$terminal_id"
}

sandbox_terminal_list() {
    local session_id="$1"
    sandbox_api_get "/v1/sessions/$session_id/terminals"
}

sandbox_terminal_read() {
    local session_id="$1"
    local terminal_id="$2"
    sandbox_api_get "/v1/sessions/$session_id/terminals/$terminal_id/read"
}

sandbox_terminal_websocket() {
    local session_id="$1"
    local terminal_id="$2"

    local ws_url
    ws_url=$(sandbox_terminal_connect "$session_id" "$terminal_id")

    if [[ -z "$ws_url" ]]; then
        _sandbox_log "ERROR" "Failed to get websocket URL"
        return 1
    fi

    echo "$ws_url"
}