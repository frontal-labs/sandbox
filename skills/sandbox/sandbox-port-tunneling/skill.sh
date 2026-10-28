#!/usr/bin/env bash
# Frontal Sandbox Port Tunneling - Port forwarding and public URLs

set -euo pipefail

SANDBOX_TUNNEL_VERSION="1.0.0"

sandbox_tunnel_create() {
    local session_id="$1"
    local port="$2"
    local protocol="${3:-http}"
    local host="${4:-localhost}"

    local payload
    payload=$(jq -n \
        --argjson port "$port" \
        --arg protocol "$protocol" \
        --arg host "$host" \
        '{port: $port, protocol: $protocol, host: $host}')

    sandbox_api_post "/v1/sessions/$session_id/tunnels" "$payload"
}

sandbox_tunnel_list() {
    local session_id="$1"
    sandbox_api_get "/v1/sessions/$session_id/tunnels"
}

sandbox_tunnel_get() {
    local session_id="$1"
    local tunnel_id="$2"
    sandbox_api_get "/v1/sessions/$session_id/tunnels/$tunnel_id"
}

sandbox_tunnel_delete() {
    local session_id="$1"
    local tunnel_id="$2"
    sandbox_api_delete "/v1/sessions/$session_id/tunnels/$tunnel_id"
}

sandbox_tunnel_update() {
    local session_id="$1"
    local tunnel_id="$2"
    local updates="$3"
    sandbox_api_patch "/v1/sessions/$session_id/tunnels/$tunnel_id" "$updates"
}

sandbox_expose_port() {
    local session_id="$1"
    local port="$2"
    local protocol="${3:-http}"

    sandbox_tunnel_create "$session_id" "$port" "$protocol" "localhost"
}

sandbox_tunnel_wait_ready() {
    local session_id="$1"
    local tunnel_id="$2"
    local timeout="${3:-30}"
    local interval=2
    local elapsed=0

    while (( elapsed < timeout )); do
        local tunnel
        tunnel=$(sandbox_tunnel_get "$session_id" "$tunnel_id")
        local status
        status=$(echo "$tunnel" | jq -r '.status // "unknown"')

        if [[ "$status" == "ready" ]]; then
            echo "$tunnel"
            return 0
        elif [[ "$status" == "failed" ]]; then
            _sandbox_log "ERROR" "Tunnel failed: $tunnel"
            return 1
        fi

        sleep "$interval"
        elapsed=$((elapsed + interval))
    done

    _sandbox_log "ERROR" "Timeout waiting for tunnel $tunnel_id"
    return 1
}

sandbox_get_public_url() {
    local session_id="$1"
    local port="$2"
    local protocol="${3:-http}"

    local tunnel
    tunnel=$(sandbox_expose_port "$session_id" "$port" "$protocol")
    local tunnel_id
    tunnel_id=$(echo "$tunnel" | jq -r '.id')

    tunnel=$(sandbox_tunnel_wait_ready "$session_id" "$tunnel_id" 30)
    echo "$tunnel" | jq -r '.url // empty'
}