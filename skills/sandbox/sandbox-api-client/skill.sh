#!/usr/bin/env bash
# Frontal Sandbox API Client - Core HTTP client for Frontal Sandbox API

set -euo pipefail

: "${SANDBOX_API_URL:?SANDBOX_API_URL must be set}"
: "${SANDBOX_API_KEY:?SANDBOX_API_KEY must be set}"

SANDBOX_API_CLIENT_VERSION="1.0.0"
SANDBOX_MAX_RETRIES=3
SANDBOX_RETRY_DELAY=1

_sandbox_log() {
    local level="$1"
    shift
    echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] [$level] sandbox-api-client: $*" >&2
}

_sandbox_retry() {
    local max_retries="$1"
    local delay="$2"
    shift 2
    local attempt=1
    while true; do
        if "$@"; then
            return 0
        fi
        local exit_code=$?
        if (( attempt >= max_retries )); then
            _sandbox_log "ERROR" "Command failed after $attempt attempts: $*"
            return $exit_code
        fi
        _sandbox_log "WARN" "Attempt $attempt failed, retrying in ${delay}s..."
        sleep "$delay"
        delay=$((delay * 2))
        ((attempt++))
    done
}

sandbox_api_request() {
    local method="$1"
    local endpoint="$2"
    local data="${3:-}"
    local extra_headers="${4:-}"

    local url="${SANDBOX_API_URL%/}${endpoint}"
    local headers=(-H "Authorization: Bearer $SANDBOX_API_KEY" -H "Content-Type: application/json" -H "Accept: application/json" -H "User-Agent: sandbox-api-client/$SANDBOX_API_CLIENT_VERSION")

    if [[ -n "$extra_headers" ]]; then
        while IFS= read -r line; do
            headers+=(-H "$line")
        done <<< "$extra_headers"
    fi

    local curl_args=(-sS -X "$method" "${headers[@]}" --url "$url")
    if [[ -n "$data" ]]; then
        curl_args+=(--data "$data")
    fi

    _sandbox_retry "$SANDBOX_MAX_RETRIES" "$SANDBOX_RETRY_DELAY" \
        curl "${curl_args[@]}" \
        --write-out '\n%{http_code}' \
        --output /tmp/sandbox_response.json
}

sandbox_api_get() {
    local endpoint="$1"
    local params="${2:-}"
    if [[ -n "$params" ]]; then
        endpoint="${endpoint}?${params}"
    fi
    sandbox_api_request "GET" "$endpoint"
}

sandbox_api_post() {
    local endpoint="$1"
    local data="${2:-}"
    sandbox_api_request "POST" "$endpoint" "$data"
}

sandbox_api_put() {
    local endpoint="$1"
    local data="${2:-}"
    sandbox_api_request "PUT" "$endpoint" "$data"
}

sandbox_api_patch() {
    local endpoint="$1"
    local data="${2:-}"
    sandbox_api_request "PATCH" "$endpoint" "$data"
}

sandbox_api_delete() {
    local endpoint="$1"
    sandbox_api_request "DELETE" "$endpoint"
}

_sandbox_parse_response() {
    local response_file="/tmp/sandbox_response.json"
    local http_code
    http_code=$(tail -n1 "$response_file")
    local body
    body=$(head -n -1 "$response_file")

    case "$http_code" in
        2[0-9][0-9])
            echo "$body"
            ;;
        401)
            _sandbox_log "ERROR" "Authentication failed - check SANDBOX_API_KEY"
            echo "{\"error\": \"unauthorized\", \"status\": 401}" >&2
            return 1
            ;;
        429)
            _sandbox_log "WARN" "Rate limited"
            return 2
            ;;
        4[0-9][0-9])
            _sandbox_log "ERROR" "Client error $http_code: $body"
            echo "{\"error\": \"client_error\", \"status\": $http_code, \"body\": $body}" >&2
            return 1
            ;;
        5[0-9][0-9])
            _sandbox_log "ERROR" "Server error $http_code: $body"
            echo "{\"error\": \"server_error\", \"status\": $http_code, \"body\": $body}" >&2
            return 1
            ;;
        *)
            _sandbox_log "ERROR" "Unexpected status $http_code: $body"
            echo "{\"error\": \"unknown\", \"status\": $http_code, \"body\": $body}" >&2
            return 1
            ;;
    esac
}

sandbox_api_request() {
    local method="$1"
    local endpoint="$2"
    local data="${3:-}"
    local extra_headers="${4:-}"

    local url="${SANDBOX_API_URL%/}${endpoint}"
    local headers=(-H "Authorization: Bearer $SANDBOX_API_KEY" -H "Content-Type: application/json" -H "Accept: application/json" -H "User-Agent: sandbox-api-client/$SANDBOX_API_CLIENT_VERSION")

    if [[ -n "$extra_headers" ]]; then
        while IFS= read -r line; do
            headers+=(-H "$line")
        done <<< "$extra_headers"
    fi

    local curl_args=(-sS -X "$method" "${headers[@]}" --url "$url")
    if [[ -n "$data" ]]; then
        curl_args+=(--data "$data")
    fi

    _sandbox_retry "$SANDBOX_MAX_RETRIES" "$SANDBOX_RETRY_DELAY" \
        curl "${curl_args[@]}" \
        --write-out '\n%{http_code}' \
        --output /tmp/sandbox_response.json

    _sandbox_parse_response
}