#!/usr/bin/env bash
# Frontal Sandbox Preview - Preview URL management

set -euo pipefail

SANDBOX_PREVIEW_VERSION="1.0.0"

sandbox_preview_create() {
    local session_id="$1"
    local port="$2"
    local options="${3:-{}}"

    local payload
    payload=$(jq -n \
        --argjson port "$port" \
        --argjson opts "$options" \
        '{port: $port, options: $opts}')

    sandbox_api_post "/v1/sessions/$session_id/previews" "$payload"
}

sandbox_preview_list() {
    local session_id="$1"
    sandbox_api_get "/v1/sessions/$session_id/previews"
}

sandbox_preview_get() {
    local session_id="$1"
    local preview_id="$2"
    sandbox_api_get "/v1/sessions/$session_id/previews/$preview_id"
}

sandbox_preview_delete() {
    local session_id="$1"
    local preview_id="$2"
    sandbox_api_delete "/v1/sessions/$session_id/previews/$preview_id"
}

sandbox_preview_update() {
    local session_id="$1"
    local preview_id="$2"
    local updates="$3"
    sandbox_api_patch "/v1/sessions/$session_id/previews/$preview_id" "$updates"
}

sandbox_auto_preview() {
    local session_id="$1"
    local command="$2"
    local expected_port="$3"
    local options="${4:-{}}"

    local payload
    payload=$(jq -n \
        --arg cmd "$command" \
        --argjson port "$expected_port" \
        --argjson opts "$options" \
        '{command: $cmd, expected_port: $port, options: $opts}')

    sandbox_api_post "/v1/sessions/$session_id/previews/auto" "$payload"
}

sandbox_preview_wait_ready() {
    local session_id="$1"
    local preview_id="$2"
    local timeout="${3:-30}"
    local interval=2
    local elapsed=0

    while (( elapsed < timeout )); do
        local preview
        preview=$(sandbox_preview_get "$session_id" "$preview_id")
        local status
        status=$(echo "$preview" | jq -r '.status // "unknown"')

        if [[ "$status" == "ready" ]]; then
            echo "$preview"
            return 0
        elif [[ "$status" == "failed" ]]; then
            _sandbox_log "ERROR" "Preview failed: $preview"
            return 1
        fi

        sleep "$interval"
        elapsed=$((elapsed + interval))
    done

    _sandbox_log "ERROR" "Timeout waiting for preview $preview_id"
    return 1
}

sandbox_preview_get_url() {
    local session_id="$1"
    local port="$2"
    local options="${3:-{}}"

    local preview
    preview=$(sandbox_preview_create "$session_id" "$port" "$options")
    local preview_id
    preview_id=$(echo "$preview" | jq -r '.id')

    preview=$(sandbox_preview_wait_ready "$session_id" "$preview_id" 30)
    echo "$preview" | jq -r '.url // empty'
}