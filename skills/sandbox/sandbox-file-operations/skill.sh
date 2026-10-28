#!/usr/bin/env bash
# Frontal Sandbox File Operations - File system operations in sandbox sessions

set -euo pipefail

SANDBOX_FILE_OPS_VERSION="1.0.0"

sandbox_file_read() {
    local session_id="$1"
    local path="$2"
    sandbox_api_get "/v1/sessions/$session_id/files$(printf '%s' "$path" | jq -sRr @uri)"
}

sandbox_file_write() {
    local session_id="$1"
    local path="$2"
    local content="$3"
    local mode="${4:-0644}"

    local payload
    payload=$(jq -n --arg content "$content" --arg mode "$mode" '{content: $content, mode: $mode}')
    sandbox_api_put "/v1/sessions/$session_id/files$(printf '%s' "$path" | jq -sRr @uri)" "$payload"
}

sandbox_file_list() {
    local session_id="$1"
    local path="${2:-/workspace}"
    local recursive="${3:-false}"

    local params="recursive=$recursive"
    sandbox_api_get "/v1/sessions/$session_id/files$(printf '%s' "$path" | jq -sRr @uri)" "$params"
}

sandbox_file_upload() {
    local session_id="$1"
    local local_path="$2"
    local remote_path="$3"

    if [[ ! -f "$local_path" ]]; then
        _sandbox_log "ERROR" "Local file not found: $local_path"
        return 1
    fi

    local content
    content=$(base64 -w0 < "$local_path")
    local payload
    payload=$(jq -n --arg content "$content" --arg encoding "base64" '{content: $content, encoding: $encoding}')
    sandbox_api_put "/v1/sessions/$session_id/files$(printf '%s' "$remote_path" | jq -sRr @uri)" "$payload"
}

sandbox_file_download() {
    local session_id="$1"
    local remote_path="$2"
    local local_path="$3"

    local response
    response=$(sandbox_api_get "/v1/sessions/$session_id/files$(printf '%s' "$remote_path" | jq -sRr @uri)?download=true")

    local content
    content=$(echo "$response" | jq -r '.content // empty')
    local encoding
    encoding=$(echo "$response" | jq -r '.encoding // "utf-8"')

    if [[ "$encoding" == "base64" ]]; then
        echo "$content" | base64 -d > "$local_path"
    else
        echo "$content" > "$local_path"
    fi
}

sandbox_file_delete() {
    local session_id="$1"
    local path="$2"
    local recursive="${3:-false}"

    local params="recursive=$recursive"
    sandbox_api_delete "/v1/sessions/$session_id/files$(printf '%s' "$path" | jq -sRr @uri)?$params"
}

sandbox_file_mkdir() {
    local session_id="$1"
    local path="$2"
    local recursive="${3:-true}"

    local payload
    payload=$(jq -n --argjson recursive "$recursive" '{recursive: $recursive}')
    sandbox_api_post "/v1/sessions/$session_id/files$(printf '%s' "$path" | jq -sRr @uri)/mkdir" "$payload"
}

sandbox_file_exists() {
    local session_id="$1"
    local path="$2"

    local response
    response=$(sandbox_api_get "/v1/sessions/$session_id/files$(printf '%s' "$path" | jq -sRr @uri)/stat" 2>/dev/null) || return 1
    echo "$response" | jq -e '.exists == true' >/dev/null
}

sandbox_file_stat() {
    local session_id="$1"
    local path="$2"
    sandbox_api_get "/v1/sessions/$session_id/files$(printf '%s' "$path" | jq -sRr @uri)/stat"
}

sandbox_dir_sync() {
    local session_id="$1"
    local local_dir="$2"
    local remote_dir="$3"
    local exclude_patterns="${4:-}"

    if [[ ! -d "$local_dir" ]]; then
        _sandbox_log "ERROR" "Local directory not found: $local_dir"
        return 1
    fi

    find "$local_dir" -type f | while IFS= read -r file; do
        local rel_path="${file#$local_dir/}"
        local remote_path="$remote_dir/$rel_path"

        if [[ -n "$exclude_patterns" ]]; then
            local skip=false
            for pattern in $exclude_patterns; do
                if [[ "$rel_path" == $pattern ]]; then
                    skip=true
                    break
                fi
            done
            if [[ "$skip" == true ]]; then
                continue
            fi
        fi

        _sandbox_log "INFO" "Syncing: $rel_path"
        sandbox_file_upload "$session_id" "$file" "$remote_path"
    done
}