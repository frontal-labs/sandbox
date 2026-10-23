#!/usr/bin/env bash
# Frontal Sandbox Session Manager - Session lifecycle management

set -euo pipefail

SANDBOX_SESSION_MANAGER_VERSION="1.0.0"

sandbox_session_create() {
    local image="$1"
    local timeout="${2:-300}"
    local env_vars="${3:-{}}"
    local resources="${4:-{}}"

    local payload
    payload=$(jq -n \
        --arg image "$image" \
        --argjson timeout "$timeout" \
        --argjson env "$env_vars" \
        --argjson resources "$resources" \
        '{image: $image, timeout: $timeout, env: $env, resources: $resources}')

    sandbox_api_post "/v1/sessions" "$payload"
}

sandbox_session_get() {
    local session_id="$1"
    sandbox_api_get "/v1/sessions/$session_id"
}

sandbox_session_list() {
    local status="${1:-}"
    local limit="${2:-50}"
    local offset="${3:-0}"

    local params="limit=$limit&offset=$offset"
    if [[ -n "$status" ]]; then
        params="${params}&status=$status"
    fi
    sandbox_api_get "/v1/sessions" "$params"
}

sandbox_session_update() {
    local session_id="$1"
    local updates="$2"
    sandbox_api_patch "/v1/sessions/$session_id" "$updates"
}

sandbox_session_delete() {
    local session_id="$1"
    sandbox_api_delete "/v1/sessions/$session_id"
}

sandbox_session_wait_ready() {
    local session_id="$1"
    local timeout="${2:-120}"
    local interval=5
    local elapsed=0

    while (( elapsed < timeout )); do
        local session
        session=$(sandbox_session_get "$session_id")
        local status
        status=$(echo "$session" | jq -r '.status // "unknown"')

        case "$status" in
            ready)
                echo "$session"
                return 0
                ;;
            failed|error)
                echo "Session failed: $session" >&2
                return 1
                ;;
            creating|starting)
                sleep "$interval"
                elapsed=$((elapsed + interval))
                ;;
            *)
                _sandbox_log "WARN" "Unknown session status: $status"
                sleep "$interval"
                elapsed=$((elapsed + interval))
                ;;
        esac
    done

    _sandbox_log "ERROR" "Timeout waiting for session $session_id to be ready"
    return 1
}

sandbox_session_extend() {
    local session_id="$1"
    local additional_seconds="$2"
    sandbox_api_post "/v1/sessions/$session_id/extend" "{\"seconds\": $additional_seconds}"
}

sandbox_session_logs() {
    local session_id="$1"
    local follow="${2:-false}"
    local params="follow=$follow"
    sandbox_api_get "/v1/sessions/$session_id/logs" "$params"
}