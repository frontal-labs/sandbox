# Frontal Sandbox Port Tunneling Skill

## Overview
Manage port forwarding and tunneling for sandbox sessions. Expose internal ports to public URLs or local machine.

## When to Use
- Exposing web servers running in sandbox
- Accessing databases/services in sandbox
- Sharing preview URLs
- Remote debugging

## Dependencies
- sandbox-api-client
- sandbox-session-manager

## Functions

### `sandbox_tunnel_create(session_id, port, protocol, host)`
Create a tunnel for a port.

### `sandbox_tunnel_list(session_id)`
List active tunnels.

### `sandbox_tunnel_get(session_id, tunnel_id)`
Get tunnel details.

### `sandbox_tunnel_delete(session_id, tunnel_id)`
Close a tunnel.

### `sandbox_tunnel_update(session_id, tunnel_id, updates)`
Update tunnel configuration.

### `sandbox_expose_port(session_id, port, protocol)`
Quick expose a port (creates tunnel with defaults).

## Protocols
- http: HTTP/HTTPS traffic
- tcp: Raw TCP
- ws: WebSocket
- grpc: gRPC

## Example Usage
```bash
source "$SANDBOX_SKILLS_DIR/sandbox-port-tunneling/skill.sh"

# Expose port 3000 for a Next.js dev server
tunnel=$(sandbox_expose_port "$session_id" 3000 "http")
public_url=$(echo "$tunnel" | jq -r '.url')
echo "App available at: $public_url"

# Custom tunnel
tunnel=$(sandbox_tunnel_create "$session_id" 5432 "tcp" "0.0.0.0")
```