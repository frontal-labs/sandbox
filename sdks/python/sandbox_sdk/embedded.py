"""
Embedded sandbox mode for Python SDK.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass

import httpx

from .client import Client
from .errors import Result, SandboxError, invalid_state, unavailable
from .types import ExecResult, SessionConfig, Snapshot


@dataclass(frozen=True)
class EmbeddedSandboxConfig(SessionConfig):
  server_path: str = "sandbox-server"


class EmbeddedSandbox:
  def __init__(self, client: Client, session_id: str) -> None:
    self._client = client
    self._session_id: str | None = session_id
    self._server_process: subprocess.Popen[bytes] | None = None

  @classmethod
  async def new(cls, config: EmbeddedSandboxConfig) -> Result[EmbeddedSandbox]:
    session_config = SessionConfig(
      tenant_id=config.tenant_id,
      tier_preference=config.tier_preference,
      limits=config.limits,
      network=config.network,
      env=config.env,
      cwd=config.cwd,
      initial_snapshot=config.initial_snapshot,
      secrets=config.secrets,
      labels=config.labels,
    )

    client = Client.from_env()

    if client.base_url == "http://localhost:8080":
      try:
        async with httpx.AsyncClient(timeout=2.0) as http_client:
          await http_client.get("http://localhost:8080/health")
      except Exception:
        return unavailable(
          "No sandbox server running. Start 'sandbox-server' or set SANDBOX_BASE_URL."
        )

    session_result = await client.create_session(session_config)
    if isinstance(session_result, SandboxError):
      return session_result

    return cls(client, session_result)

  async def exec(self, command: str) -> Result[ExecResult]:
    if not self._session_id:
      return invalid_state("No active session")

    from .types import ExecRequest

    return await self._client.exec(
      self._session_id,
      ExecRequest(argv=["sh", "-c", command], env={}, stream=False),
    )

  async def snapshot(self) -> Result[Snapshot]:
    if not self._session_id:
      return invalid_state("No active session")

    return await self._client.snapshot(self._session_id)

  async def destroy(self) -> Result[None]:
    if not self._session_id:
      return invalid_state("No active session")

    result = await self._client.destroy_session(self._session_id)
    self._session_id = None
    return result

  @property
  def session_id(self) -> str | None:
    return self._session_id
