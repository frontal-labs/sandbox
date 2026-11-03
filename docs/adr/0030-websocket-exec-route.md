# 0030: WebSocket exec route

## Status
Accepted. The cancellation decision below is superseded by ADR 0032.

## Date
2026-11-03

## Context
PLAN §4.15 places the interactive WebSocket at `GET /v1/sessions/{id}/ws`. The server had a WebSocket exec handler, but it was routed as a GET upgrade on `/v1/sessions/{id}/exec`, and it read the exec request from a JSON body on that GET. Browsers and most WebSocket clients cannot send a body on the upgrade request, so the handler could not be used from them. It had no end-to-end test. The exec logic it used was also duplicated from the NDJSON handler.

## Decision
- The route is `GET /v1/sessions/{id}/ws`, as §4.15 specifies. The GET on `/exec` is removed.
- The client's first message is the JSON `ExecRequest`. The server then sends each `ExecEvent` as a text message, and closes with a close frame after the final `exit` or `error`.
- A first message that is not a valid request, or an empty `argv`, is answered with an `error` event (`INVALID_ARGUMENT`) and a close frame. Nothing runs.
- The session gate is taken after the request is read, so an idle connection does not hold the session.
- The command runs in the background, as in the NDJSON path. Both handlers now share one starter, `start_exec`.
- Stdin beyond the request's `stdin` field and signals are not part of this route yet. Later client messages are not read in this decision. Closing the socket does not cancel the command. Superseded by ADR 0032, which adds `cancel` and cancellation on disconnect.

## Consequences
### Positive
- Any WebSocket client can run a command with the plan's route and no request body on the upgrade.
- The exec logic lives in one place, and the WebSocket path has end-to-end tests over a real TCP socket.

### Negative
- Clients of the old `GET /exec` upgrade must move to `/ws`. No SDK in this repository used it.
- Interactive stdin and signals are still missing, so the route covers exec and streaming only.

### Risks
- A client that disconnects leaves its command running to completion or its timeout. Superseded by ADR 0032.

## Alternatives Considered
- Keep the JSON body on the GET upgrade. Rejected: it cannot be used from browsers.
- Put the request in the query string. Rejected: commands can be long, and query strings are logged.

## References
- PLAN §4.15 (routes), ADR 0020 (NDJSON streaming), ADR 0028 (per-byte streaming).
