"""
Type definitions for the sandbox SDK.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal, TypedDict

import ulid

Tier = Literal["isolate", "vm"]
TierPreference = Literal["auto", "isolate", "vm"]
SessionStatus = Literal["creating", "running", "sleeping", "promoting", "destroyed", "error"]


@dataclass(frozen=True)
class ResourceLimits:
  memory_bytes: int | None = None
  cpu_ms_per_sec: int | None = None
  wall_timeout_secs: int | None = None
  max_file_size: int | None = None
  max_total_size: int | None = None
  max_inodes: int | None = None
  max_path_depth: int | None = None
  max_processes: int | None = None
  max_open_files: int | None = None


class NetworkPolicyRule(TypedDict):
  action: Literal["allow", "deny"]
  host_pattern: str
  ports: list[int] | None


@dataclass(frozen=True)
class NetworkPolicy:
  default_deny: bool = True
  rules: list[NetworkPolicyRule] = field(default_factory=list)

  @staticmethod
  def allow_all() -> NetworkPolicy:
    return NetworkPolicy(default_deny=False, rules=[])

  def add_rule(self, rule: NetworkPolicyRule) -> NetworkPolicy:
    return NetworkPolicy(default_deny=self.default_deny, rules=[*self.rules, rule])


@dataclass(frozen=True)
class SecretEntry:
  value: str
  hosts: list[str]


@dataclass(frozen=True)
class SecretsConfig:
  secrets: dict[str, SecretEntry]


@dataclass(frozen=True)
class SessionConfig:
  tenant_id: str
  tier_preference: TierPreference = "auto"
  limits: ResourceLimits = field(default_factory=ResourceLimits)
  network: NetworkPolicy = field(default_factory=NetworkPolicy)
  env: dict[str, str] = field(default_factory=dict)
  cwd: str = "/work"
  initial_snapshot: str | None = None
  secrets: SecretsConfig | None = None
  labels: dict[str, str] = field(default_factory=dict)

  @staticmethod
  def new(tenant_id: str) -> SessionConfig:
    return SessionConfig(tenant_id=tenant_id)

  def with_tier(self, tier: TierPreference) -> SessionConfig:
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=tier,
      limits=self.limits,
      network=self.network,
      env=self.env,
      cwd=self.cwd,
      initial_snapshot=self.initial_snapshot,
      secrets=self.secrets,
      labels=self.labels,
    )

  def with_limits(self, limits: ResourceLimits) -> SessionConfig:
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=self.tier_preference,
      limits=limits,
      network=self.network,
      env=self.env,
      cwd=self.cwd,
      initial_snapshot=self.initial_snapshot,
      secrets=self.secrets,
      labels=self.labels,
    )

  def with_network(self, network: NetworkPolicy) -> SessionConfig:
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=self.tier_preference,
      limits=self.limits,
      network=network,
      env=self.env,
      cwd=self.cwd,
      initial_snapshot=self.initial_snapshot,
      secrets=self.secrets,
      labels=self.labels,
    )

  def with_env(self, env: dict[str, str]) -> SessionConfig:
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=self.tier_preference,
      limits=self.limits,
      network=self.network,
      env=env,
      cwd=self.cwd,
      initial_snapshot=self.initial_snapshot,
      secrets=self.secrets,
      labels=self.labels,
    )

  def with_cwd(self, cwd: str) -> SessionConfig:
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=self.tier_preference,
      limits=self.limits,
      network=self.network,
      env=self.env,
      cwd=cwd,
      initial_snapshot=self.initial_snapshot,
      secrets=self.secrets,
      labels=self.labels,
    )

  def with_snapshot(self, snapshot: str) -> SessionConfig:
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=self.tier_preference,
      limits=self.limits,
      network=self.network,
      env=self.env,
      cwd=self.cwd,
      initial_snapshot=snapshot,
      secrets=self.secrets,
      labels=self.labels,
    )

  def with_secrets(self, secrets: SecretsConfig) -> SessionConfig:
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=self.tier_preference,
      limits=self.limits,
      network=self.network,
      env=self.env,
      cwd=self.cwd,
      initial_snapshot=self.initial_snapshot,
      secrets=secrets,
      labels=self.labels,
    )

  def with_label(self, key: str, value: str) -> SessionConfig:
    new_labels = {**self.labels, key: value}
    return SessionConfig(
      tenant_id=self.tenant_id,
      tier_preference=self.tier_preference,
      limits=self.limits,
      network=self.network,
      env=self.env,
      cwd=self.cwd,
      initial_snapshot=self.initial_snapshot,
      secrets=self.secrets,
      labels=new_labels,
    )


@dataclass(frozen=True)
class SessionState:
  id: str
  tenant_id: str
  tier: Tier
  status: SessionStatus
  config_hash: str
  vfs_root: str | None
  created_at: str
  updated_at: str
  labels: dict[str, str]
  # Pinned checkpoints, as GET reports them. Absent from responses that omit it.
  checkpoints: dict[str, object] | None = None
  # Whether a command the isolate tier cannot run may move this session to a Tier 2 VM.
  allow_vm: bool = False


@dataclass(frozen=True)
class ExecRequest:
  argv: list[str]
  env: dict[str, str] = field(default_factory=dict)
  cwd: str | None = None
  stdin: bytes | None = None
  timeout_ms: int | None = None
  stream: bool = False


@dataclass(frozen=True)
class ExecResult:
  exit_code: int
  stdout: bytes
  stderr: bytes
  cpu_ms: int
  wall_ms: int


@dataclass(frozen=True)
class Snapshot:
  id: str
  parent: str | None
  vfs_root: str
  env: dict[str, str]
  cwd: str
  tier: Tier
  runtime_blob: str | None
  created_at: str
  labels: dict[str, str]


def generate_id() -> str:
  return str(ulid.new())


def nil_id() -> str:
  return str(ulid.from_int(0))
