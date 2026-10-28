#!/usr/bin/env bash
# Frontal Sandbox Lifecycle - Templates, snapshots, and lifecycle management

set -euo pipefail

SANDBOX_LIFECYCLE_VERSION="1.0.0"

sandbox_lifecycle_create_from_template() {
    local template_name="$1"
    local overrides="${2:-{}}"

    local payload
    payload=$(jq -n \
        --arg template "$template_name" \
        --argjson overrides "$overrides" \
        '{template: $template, overrides: $overrides}')

    sandbox_api_post "/v1/sessions/from-template" "$payload"
}

sandbox_lifecycle_snapshot() {
    local session_id="$1"
    local name="$2"
    local description="${3:-}"

    local payload
    payload=$(jq -n \
        --arg name "$name" \
        --arg desc "$description" \
        '{name: $name, description: $desc}')

    sandbox_api_post "/v1/sessions/$session_id/snapshots" "$payload"
}

sandbox_lifecycle_restore() {
    local snapshot_id="$1"
    local options="${2:-{}}"

    local payload
    payload=$(jq -n --argjson opts "$options" '{options: $opts}')

    sandbox_api_post "/v1/snapshots/$snapshot_id/restore" "$payload"
}

sandbox_lifecycle_list_templates() {
    sandbox_api_get "/v1/templates"
}

sandbox_lifecycle_list_snapshots() {
    local session_id="$1"
    sandbox_api_get "/v1/sessions/$session_id/snapshots"
}

sandbox_lifecycle_delete_snapshot() {
    local snapshot_id="$1"
    sandbox_api_delete "/v1/snapshots/$snapshot_id"
}

sandbox_lifecycle_set_ttl() {
    local session_id="$1"
    local ttl_seconds="$2"

    local payload
    payload=$(jq -n --argjson ttl "$ttl_seconds" '{ttl: $ttl}')
    sandbox_api_post "/v1/sessions/$session_id/ttl" "$payload"
}

sandbox_lifecycle_extend_ttl() {
    local session_id="$1"
    local additional_seconds="$2"

    local payload
    payload=$(jq -n --argjson ext "$additional_seconds" '{extension: $ext}')
    sandbox_api_post "/v1/sessions/$session_id/ttl/extend" "$payload"
}

sandbox_lifecycle_get_events() {
    local session_id="$1"
    sandbox_api_get "/v1/sessions/$session_id/events"
}

sandbox_lifecycle_get_template() {
    local template_name="$1"
    sandbox_api_get "/v1/templates/$template_name"
}

sandbox_lifecycle_create_template() {
    local name="$1"
    local description="$2"
    local config="$3"

    local payload
    payload=$(jq -n \
        --arg name "$name" \
        --arg desc "$description" \
        --argjson config "$config" \
        '{name: $name, description: $desc, config: $config}')

    sandbox_api_post "/v1/templates" "$payload"
}

sandbox_lifecycle_wait_for_state() {
    local session_id="$1"
    local target_state="$2"
    local timeout="${3:-120}"
    local interval=5
    local elapsed=0

    while (( elapsed < timeout )); do
        local session
        session=$(sandbox_session_get "$session_id")
        local state
        state=$(echo "$session" | jq -r '.status // "unknown"')

        if [[ "$state" == "$target_state" ]]; then
            echo "$session"
            return 0
        elif [[ "$state" == "failed" || "$state" == "error" ]]; then
            _sandbox_log "ERROR" "Session entered failed state: $session"
            return 1
        fi

        sleep "$interval"
        elapsed=$((elapsed + interval))
    done

    _sandbox_log "ERROR" "Timeout waiting for session $session_id to reach $target_state"
    return 1
}