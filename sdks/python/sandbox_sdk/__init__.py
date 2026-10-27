"""
Sandbox SDK for Python.

A client library for interacting with the sandbox execution system.
"""

from .client import Client, ClientConfig
from .embedded import EmbeddedSandbox, EmbeddedSandboxConfig
from .tier1 import Tier1Client
from .errors import (
    ErrorCode,
    Result,
    SandboxError,
    already_exists,
    cancelled,
    cas_error,
    internal,
    invalid_argument,
    invalid_state,
    is_sandbox_error,
    network_error,
    not_found,
    permission_denied,
    policy_denied,
    promotion_failed,
    resource_exhausted,
    runtime_error,
    shell_error,
    snapshot_failed,
    timeout,
    unavailable,
    unsupported,
    vfs_error,
    vm_boot_failed,
    wasm_error,
)
from .types import (
    ExecRequest,
    ExecResult,
    NetworkPolicy,
    NetworkPolicyRule,
    ResourceLimits,
    SecretEntry,
    SecretsConfig,
    SessionConfig,
    SessionState,
    SessionStatus,
    Snapshot,
    Tier,
    TierPreference,
    generate_id,
    nil_id,
)

__version__ = "0.0.0"

__all__ = [
    # Types
    "Tier",
    "TierPreference",
    "SessionStatus",
    "ResourceLimits",
    "NetworkPolicy",
    "NetworkPolicyRule",
    "SecretEntry",
    "SecretsConfig",
    "SessionConfig",
    "SessionState",
    "ExecRequest",
    "ExecResult",
    "Snapshot",
    "generate_id",
    "nil_id",
    # Errors
    "ErrorCode",
    "SandboxError",
    "internal",
    "invalid_argument",
    "not_found",
    "already_exists",
    "permission_denied",
    "resource_exhausted",
    "unavailable",
    "timeout",
    "cancelled",
    "invalid_state",
    "unsupported",
    "policy_denied",
    "promotion_failed",
    "snapshot_failed",
    "vm_boot_failed",
    "network_error",
    "cas_error",
    "vfs_error",
    "shell_error",
    "runtime_error",
    "wasm_error",
    "is_sandbox_error",
    "Result",
    # Client
    "Client",
    "ClientConfig",
    # Embedded
    "EmbeddedSandbox",
    "EmbeddedSandboxConfig",
    # Tier 1 session tools
    "Tier1Client",
]