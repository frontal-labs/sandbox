"""
HTTP client for the sandbox SDK.
"""

from __future__ import annotations

import os
from collections.abc import AsyncGenerator
from dataclasses import asdict, dataclass, is_dataclass
from typing import Any, TypeAlias, cast, get_args
from urllib.parse import quote

import httpx

from .errors import (
  ErrorCode,
  Result,
  SandboxError,
  internal,
  invalid_argument,
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

# Every error code the SDK knows, as the ErrorCode literal lists them.
KNOWN_ERROR_CODES: frozenset[str] = frozenset(get_args(ErrorCode))

# A decoded JSON response: an object, an array, or no body (HTTP 204).
JsonValue: TypeAlias = dict[str, Any] | list[Any] | None


def _encode_path(path: str) -> str:
  """URL-encode a filesystem path, preserving slashes.

  Dot segments are refused: an HTTP client resolves them before sending, so the
  request could reach another route.
  """
  segments = path.strip().split("/")
  for segment in segments:
    if segment in (".", ".."):
      raise invalid_argument(f"path segment {segment!r} is not allowed")
  return "/".join(quote(segment, safe="") for segment in segments)


@dataclass(frozen=True)
class ClientConfig:
  base_url: str
  api_key: str | None = None


def _wire(value: object) -> Any:
  """Converts a dataclass tree to JSON-ready values. Bytes become integer
  arrays, which is how the server encodes byte fields. Nested dataclasses
  are converted too, so no field is left as a Python object."""
  if is_dataclass(value) and not isinstance(value, type):
    return {k: _wire(v) for k, v in asdict(value).items()}
  if isinstance(value, dict):
    return {k: _wire(v) for k, v in value.items()}
  if isinstance(value, (list, tuple)):
    return [_wire(v) for v in value]
  if isinstance(value, (bytes, bytearray)):
    return list(value)
  return value


def _exec_result(result: dict[str, Any]) -> ExecResult:
  return ExecResult(
    exit_code=result["exit_code"],
    stdout=bytes(result.get("stdout", [])),
    stderr=bytes(result.get("stderr", [])),
    cpu_ms=result.get("cpu_ms", 0),
    wall_ms=result.get("wall_ms", 0),
  )


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
    json_body: dict[str, Any] | None = None,
  ) -> Result[JsonValue]:
    url = f"{self.base_url}{path}"
    try:
      response = await self._client.request(method, url, json=json_body)
      if response.is_success:
        if response.status_code == 204:
          return None
        body: JsonValue = response.json()
        return body
      return await self._error_from_response(response)
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
      json_body=_wire(config),
    )
    if is_sandbox_error(result):
      return result
    session_id = result.get("session_id") if isinstance(result, dict) else None
    if isinstance(session_id, str):
      return session_id
    return internal("Invalid response")

  async def get_session(self, session_id: str) -> Result[SessionState]:
    result = await self._request("GET", f"/v1/sessions/{quote(session_id, safe='')}")
    if is_sandbox_error(result):
      return result
    return SessionState(**result) if isinstance(result, dict) else internal("Invalid response")

  async def destroy_session(self, session_id: str) -> Result[None]:
    result = await self._request("DELETE", f"/v1/sessions/{quote(session_id, safe='')}")
    if is_sandbox_error(result):
      return result
    return None

  async def exec(self, session_id: str, request: ExecRequest) -> Result[ExecResult]:
    result = await self._request(
      "POST",
      f"/v1/sessions/{quote(session_id, safe='')}/exec",
      json_body=_wire(request),
    )
    if is_sandbox_error(result):
      return result
    return _exec_result(result) if isinstance(result, dict) else internal("Invalid response")

  async def exec_stream(
    self, session_id: str, request: ExecRequest
  ) -> AsyncGenerator[Result[ExecResult], None]:
    # Streaming exec is not implemented on the server, so this yields one
    # UNSUPPORTED error and does not send a request.
    yield unsupported("streaming exec is not implemented yet")

  async def snapshot(self, session_id: str) -> Result[Snapshot]:
    taken = await self._request("POST", f"/v1/sessions/{quote(session_id, safe='')}/snapshot")
    if is_sandbox_error(taken):
      return taken
    if not (isinstance(taken, dict) and isinstance(taken.get("snapshot_id"), str)):
      return internal("Invalid response")
    return await self.get_snapshot(session_id, taken["snapshot_id"])

  async def sleep(self, session_id: str) -> Result[str]:
    """Puts the session to sleep and returns the id of the snapshot it wakes from."""
    result = await self._request("POST", f"/v1/sessions/{quote(session_id, safe='')}/sleep")
    if is_sandbox_error(result):
      return result
    snapshot_id = result.get("snapshot_id") if isinstance(result, dict) else None
    if isinstance(snapshot_id, str):
      return snapshot_id
    return internal("Invalid response")

  async def wake(self, session_id: str) -> Result[SessionState]:
    """Wakes a sleeping session. Its files come back; shell state starts fresh."""
    result = await self._request("POST", f"/v1/sessions/{quote(session_id, safe='')}/wake")
    if is_sandbox_error(result):
      return result
    return SessionState(**result) if isinstance(result, dict) else internal("Invalid response")

  async def list_snapshots(self, session_id: str) -> Result[list[Snapshot]]:
    result = await self._request("GET", f"/v1/sessions/{quote(session_id, safe='')}/snapshots")
    if is_sandbox_error(result):
      return result
    return (
      [Snapshot(**s) for s in result] if isinstance(result, list) else internal("Invalid response")
    )

  async def get_snapshot(self, session_id: str, snapshot_id: str) -> Result[Snapshot]:
    result = await self._request(
      "GET", f"/v1/sessions/{quote(session_id, safe='')}/snapshots/{snapshot_id}"
    )
    if is_sandbox_error(result):
      return result
    return Snapshot(**result) if isinstance(result, dict) else internal("Invalid response")

  async def fork(self, snapshot_id: str, count: int) -> Result[list[str]]:
    """Starts `count` sessions from a snapshot. Returns their IDs."""
    result = await self._request(
      "POST", f"/v1/snapshots/{snapshot_id}/fork", json_body={"count": count}
    )
    if is_sandbox_error(result):
      return result
    session_ids = result.get("session_ids") if isinstance(result, dict) else None
    if isinstance(session_ids, list) and all(isinstance(i, str) for i in session_ids):
      return cast(list[str], session_ids)
    return internal("Invalid response")

  async def read_file(self, session_id: str, path: str) -> Result[bytes]:
    """Reads a file from the session's filesystem."""
    try:
      url = f"{self.base_url}/v1/sessions/{quote(session_id, safe='')}/fs/{_encode_path(path)}"
      response = await self._client.request("GET", url)
      if response.is_success:
        return response.content
      return await self._error_from_response(response)
    except httpx.RequestError as e:
      return network_error(str(e), e)
    except Exception as e:
      if is_sandbox_error(e):
        return e
      return internal(f"Request failed: {e}", e)

  async def write_file(self, session_id: str, path: str, bytes_data: bytes) -> Result[None]:
    """Writes a file, creating it or replacing it."""
    try:
      url = f"{self.base_url}/v1/sessions/{quote(session_id, safe='')}/fs/{_encode_path(path)}"
      response = await self._client.request(
        "PUT",
        url,
        content=bytes_data,
        headers={"Content-Type": "application/octet-stream"},
      )
      if response.is_success:
        return None
      return await self._error_from_response(response)
    except httpx.RequestError as e:
      return network_error(str(e), e)
    except Exception as e:
      if is_sandbox_error(e):
        return e
      return internal(f"Request failed: {e}", e)

  async def delete_file(self, session_id: str, path: str) -> Result[None]:
    """Deletes a file or an empty directory."""
    try:
      url = f"{self.base_url}/v1/sessions/{quote(session_id, safe='')}/fs/{_encode_path(path)}"
      response = await self._client.request("DELETE", url)
      if response.is_success:
        return None
      return await self._error_from_response(response)
    except httpx.RequestError as e:
      return network_error(str(e), e)
    except Exception as e:
      if is_sandbox_error(e):
        return e
      return internal(f"Request failed: {e}", e)

  async def _error_from_response(self, response: httpx.Response) -> SandboxError:
    # The server sends {"code", "message"} with every error, so keep its code.
    error_data: Any = (
      response.json()
      if response.headers.get("content-type", "").startswith("application/json")
      else {}
    )
    code = error_data.get("code") if isinstance(error_data, dict) else None
    # A code this SDK does not know is reported as INTERNAL, so the code type stays exact.
    error_code: ErrorCode = (
      cast(ErrorCode, code) if isinstance(code, str) and code in KNOWN_ERROR_CODES else "INTERNAL"
    )
    message = error_data.get("message") if isinstance(error_data, dict) else None
    return SandboxError(
      error_code,
      message
      if isinstance(message, str)
      else f"HTTP {response.status_code}: {response.reason_phrase}",
    )
