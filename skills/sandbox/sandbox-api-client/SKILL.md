# Frontal Sandbox API Client Skill

## Overview
Core HTTP client for interacting with the Frontal Sandbox project API. Provides authenticated requests, error handling, and response parsing.

## When to Use
- Making any API call to the Sandbox backend
- Building higher-level sandbox skills that need API access
- Handling authentication and token refresh

## Configuration
```bash
export SANDBOX_API_URL="https://api.sandbox.example.com"
export SANDBOX_API_KEY="your-api-key"
```

## Functions

### `sandbox_api_request(method, endpoint, data=None, headers=None)`
Make an authenticated request to the Frontal Sandbox API.

**Parameters:**
- `method`: HTTP method (GET, POST, PUT, DELETE, PATCH)
- `endpoint`: API endpoint path (e.g., "/v1/sessions")
- `data`: Request body (JSON serializable)
- `headers`: Additional headers

**Returns:** Parsed JSON response or raises error

### `sandbox_api_get(endpoint, params=None)`
Convenience function for GET requests.

### `sandbox_api_post(endpoint, data)`
Convenience function for POST requests.

### `sandbox_api_put(endpoint, data)`
Convenience function for PUT requests.

### `sandbox_api_delete(endpoint)`
Convenience function for DELETE requests.

## Error Handling
- Retries on 429 (rate limit) with exponential backoff
- Raises `SandboxAPIError` with status code and message on failure
- Handles token refresh automatically on 401

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-api-client/skill.sh"

response=$(sandbox_api_post "/v1/sessions" '{"image": "node:20", "timeout": 300}')
session_id=$(echo "$response" | jq -r '.id')
```