#!/usr/bin/env bash
# Frontal Sandbox AI Runner - AI/ML workload execution

set -euo pipefail

SANDBOX_AI_RUNNER_VERSION="1.0.0"

sandbox_ai_run_inference() {
    local session_id="$1"
    local model="$2"
    local input="$3"
    local options="${4:-{}}"

    local payload
    payload=$(jq -n \
        --arg model "$model" \
        --argjson input "$input" \
        --argjson opts "$options" \
        '{model: $model, input: $input, options: $opts}')

    sandbox_api_post "/v1/sessions/$session_id/ai/inference" "$payload"
}

sandbox_ai_run_agent() {
    local session_id="$1"
    local agent_config="$2"
    local task="$3"
    local options="${4:-{}}"

    local payload
    payload=$(jq -n \
        --argjson config "$agent_config" \
        --arg task "$task" \
        --argjson opts "$options" \
        '{agent: $config, task: $task, options: $opts}')

    sandbox_api_post "/v1/sessions/$session_id/ai/agent" "$payload"
}

sandbox_ai_train_model() {
    local session_id="$1"
    local training_config="$2"

    sandbox_api_post "/v1/sessions/$session_id/ai/train" "$training_config"
}

sandbox_ai_evaluate() {
    local session_id="$1"
    local eval_config="$2"

    sandbox_api_post "/v1/sessions/$session_id/ai/evaluate" "$eval_config"
}

sandbox_ai_deploy_model() {
    local session_id="$1"
    local model_path="$2"
    local options="${3:-{}}"

    local payload
    payload=$(jq -n \
        --arg path "$model_path" \
        --argjson opts "$options" \
        '{model_path: $path, options: $opts}')

    sandbox_api_post "/v1/sessions/$session_id/ai/deploy" "$payload"
}

sandbox_ai_list_models() {
    local session_id="$1"
    sandbox_api_get "/v1/sessions/$session_id/ai/models"
}

sandbox_ai_pull_model() {
    local session_id="$1"
    local model_name="$2"
    local source="${3:-huggingface}"

    local payload
    payload=$(jq -n \
        --arg name "$model_name" \
        --arg src "$source" \
        '{model: $name, source: $src}')

    sandbox_api_post "/v1/sessions/$session_id/ai/models/pull" "$payload"
}

sandbox_ai_get_job_status() {
    local session_id="$1"
    local job_id="$2"
    sandbox_api_get "/v1/sessions/$session_id/ai/jobs/$job_id"
}

sandbox_ai_cancel_job() {
    local session_id="$1"
    local job_id="$2"
    sandbox_api_delete "/v1/sessions/$session_id/ai/jobs/$job_id"
}

sandbox_ai_stream_inference() {
    local session_id="$1"
    local model="$2"
    local input="$3"
    local options="${4:-{}}"

    local payload
    payload=$(jq -n \
        --arg model "$model" \
        --argjson input "$input" \
        --argjson opts "$options" \
        --argjson stream true \
        '{model: $model, input: $input, options: $opts, stream: $stream}')

    local url="${SANDBOX_API_URL%/}/v1/sessions/$session_id/ai/inference"
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