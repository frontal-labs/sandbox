# Frontal Sandbox AI Runner Skill

## Overview
Run AI/ML workloads in sandbox sessions with support for model inference, training, and agent workflows.

## When to Use
- Running LLM inference
- Fine-tuning models
- Running AI agents
- Batch inference jobs
- Model evaluation

## Dependencies
- sandbox-api-client
- sandbox-session-manager
- sandbox-file-operations
- sandbox-code-execution
- sandbox-port-tunneling

## Functions

### `sandbox_ai_run_inference(session_id, model, input, options)`
Run model inference.

### `sandbox_ai_run_agent(session_id, agent_config, task, options)`
Run an AI agent with a task.

### `sandbox_ai_train_model(session_id, training_config)`
Start model training job.

### `sandbox_ai_evaluate(session_id, eval_config)`
Run evaluation benchmark.

### `sandbox_ai_deploy_model(session_id, model_path, options)`
Deploy model as API endpoint.

### `sandbox_ai_list_models(session_id)`
List available models in session.

### `sandbox_ai_pull_model(session_id, model_name, source)`
Pull model from registry.

## Agent Config Structure
```json
{
  "type": "react|plan-and-execute|chat",
  "model": "gpt-4|claude-3|local-model",
  "tools": ["bash", "python", "web-search"],
  "max_steps": 10,
  "temperature": 0.7
}
```

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-ai-runner/skill.sh"

# Run inference
result=$(sandbox_ai_run_inference "$session_id" "llama-3-8b" '{"prompt": "Hello"}' '{"temperature": 0.7}')

# Run agent
agent_result=$(sandbox_ai_run_agent "$session_id" \
  '{"type": "react", "model": "gpt-4", "tools": ["bash", "python"]}' \
  "Build a REST API with authentication" \
  '{"max_steps": 20}')

# Deploy model
endpoint=$(sandbox_ai_deploy_model "$session_id" "/models/my-model" '{"port": 8000}')
```