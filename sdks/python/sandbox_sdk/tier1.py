"""Tier 1 session tools: checkpoints, undo, the environment, and fork with a command.

Kept apart from `Client`, so each call is one request to the /v1 API. The server
routes are documented in docs/tier1-agent-interfaces.md.
"""

from __future__ import annotations

from typing import Any

import httpx

from . import errors


class Tier1Client:
    """Client for the Tier 1 session routes. Use it as an async context manager."""

    def __init__(
        self,
        base_url: str,
        api_key: str | None = None,
        *,
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> None:
        headers = {"Authorization": f"Bearer {api_key}"} if api_key is not None else {}
        self._client = httpx.AsyncClient(
            base_url=base_url.rstrip("/"),
            headers=headers,
            timeout=httpx.Timeout(30.0),
            transport=transport,
        )

    async def __aenter__(self) -> Tier1Client:
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self._client.aclose()

    async def checkpoint(self, session_id: str) -> dict[str, Any]:
        """Pins the session's current state. Returns the root and the checkpoint usage."""
        return await self._call("POST", f"/v1/sessions/{session_id}/checkpoint")

    async def undo(self, session_id: str, steps: int = 1) -> dict[str, Any]:
        """Restores the filesystem from `steps` checkpoints ago."""
        return await self._call("POST", f"/v1/sessions/{session_id}/undo", {"steps": steps})

    async def info(self, session_id: str) -> dict[str, Any]:
        """The session's environment, as /etc/agent/env.json."""
        return await self._call("GET", f"/v1/sessions/{session_id}/info")

    async def fork_command(
        self, session_id: str, count: int, argv: list[str]
    ) -> list[dict[str, Any]]:
        """Forks the session into `count` children and runs `argv` in each."""
        body = await self._call(
            "POST", f"/v1/sessions/{session_id}/fork", {"count": count, "argv": argv}
        )
        return body["children"]

    async def _call(self, method: str, path: str, body: Any = None) -> Any:
        response = await self._client.request(method, path, json=body)
        if response.status_code >= 400:
            try:
                error = response.json()
                code = str(error.get("code", "INTERNAL")).lower()
                message = str(error.get("message", response.reason_phrase))
            except ValueError:
                code, message = "internal", response.reason_phrase
            raise getattr(errors, code, errors.internal)(message)
        return response.json()
