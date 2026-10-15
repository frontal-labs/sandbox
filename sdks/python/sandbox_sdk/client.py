"""
HTTP client for the sandbox SDK.
"""

from __future__ import annotations

import os
from collections.abc import AsyncGenerator
from dataclasses import dataclass

import httpx

from .errors import (
    Result,
    internal,
    is_sandbox_error,
    network_error,
    unsupported,
)
from .types import (
    ExecRequest,
    ExecResult,
    SessionConfig,
    SessionState,
    Snapshot,
)


@dataclass(frozen=True)
class ClientConfig:
    base_url: str
    api_key: str | None = None


class Client:
    def __init__(self, config: ClientConfig) -> None:
        self.base_url = config.base_url.rstrip("/")
        self.api_key = config.api_key
        self._client = httpx.AsyncClient(
            timeout=httpx.Timeout(30.0),
            headers={"Content-Type": "application/json"},
        )
        if self.api_key:
            self._client.headers["Authorization"] = f"Bearer {self.api_key}"

    @classmethod
    def from_env(cls) -> Client:
        base_url = os.environ.get("SANDBOX_BASE_URL", "http://localhost:8080")
        api_key = os.environ.get("SANDBOX_API_KEY")
        return cls(ClientConfig(base_url=base_url, api_key=api_key))

    async def close(self) -> None:
        await self._client.aclose()

    async def __aenter__(self) -> Client:
        return self

    async def __aexit__(self, *args: object) -> None:
        await self.close()

    async def _request(
        self,
        method: str,
        path: str,
        *,
        json_body: dict | None = None,
    ) -> Result[dict | list | None]:
        url = f"{self.base_url}{path}"
        try:
            response = await self._client.request(
                method, url, json=json_body
            )
            if response.is_success:
                if response.status_code == 204:
                    return None
                return response.json()
            error_data = response.json() if response.headers.get("content-type", "").startswith("application/json") else {}
            return internal(
                error_data.get("message", f"HTTP {response.status_code}: {response.reason_phrase}")
            )
        except httpx.RequestError as e:
            return network_error(str(e), e)
        except Exception as e:
            if is_sandbox_error(e):
                return e
            return internal(f"Request failed: {e}", e)

    async def create_session(self, config: SessionConfig) -> Result[str]:
        result = await self._request(
            "POST",
            "/v1/sessions",
            json_body=config.__dict__,
        )
        if is_sandbox_error(result):
            return result
        return result.get("id", "") if isinstance(result, dict) else str(result)

    async def get_session(self, session_id: str) -> Result[SessionState]:
        result = await self._request("GET", f"/v1/sessions/{session_id}")
        if is_sandbox_error(result):
            return result
        return SessionState(**result) if isinstance(result, dict) else internal("Invalid response")

    async def destroy_session(self, session_id: str) -> Result[None]:
        return await self._request("DELETE", f"/v1/sessions/{session_id}")

    async def exec(self, session_id: str, request: ExecRequest) -> Result[ExecResult]:
        result = await self._request(
            "POST",
            f"/v1/sessions/{session_id}/exec",
            json_body=request.__dict__,
        )
        if is_sandbox_error(result):
            return result
        return ExecResult(**result) if isinstance(result, dict) else internal("Invalid response")

    async def exec_stream(
        self, session_id: str, request: ExecRequest
    ) -> AsyncGenerator[Result[ExecResult], None]:
        # Streaming exec is not implemented on the server, so this yields one
        # UNSUPPORTED error and does not send a request.
        yield unsupported("streaming exec is not implemented yet")

    async def snapshot(self, session_id: str) -> Result[Snapshot]:
        result = await self._request(
            "POST", f"/v1/sessions/{session_id}/snapshot"
        )
        if is_sandbox_error(result):
            return result
        return Snapshot(**result) if isinstance(result, dict) else internal("Invalid response")

    async def list_snapshots(self, session_id: str) -> Result[list[Snapshot]]:
        result = await self._request("GET", f"/v1/sessions/{session_id}/snapshots")
        if is_sandbox_error(result):
            return result
        return [Snapshot(**s) for s in result] if isinstance(result, list) else internal("Invalid response")

    async def get_snapshot(self, session_id: str, snapshot_id: str) -> Result[Snapshot]:
        result = await self._request(
            "GET", f"/v1/sessions/{session_id}/snapshots/{snapshot_id}"
        )
        if is_sandbox_error(result):
            return result
        return Snapshot(**result) if isinstance(result, dict) else internal("Invalid response")
