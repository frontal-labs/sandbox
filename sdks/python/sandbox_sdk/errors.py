"""
Error definitions for the sandbox SDK.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal, TypeAlias, TypeGuard, TypeVar

ErrorCode = Literal[
    "INTERNAL",
    "INVALID_ARGUMENT",
    "NOT_FOUND",
    "ALREADY_EXISTS",
    "PERMISSION_DENIED",
    "RESOURCE_EXHAUSTED",
    "UNAVAILABLE",
    "TIMEOUT",
    "CANCELLED",
    "INVALID_STATE",
    "UNSUPPORTED",
    "POLICY_DENIED",
    "PROMOTION_FAILED",
    "SNAPSHOT_FAILED",
    "VM_BOOT_FAILED",
    "NETWORK_ERROR",
    "CAS_ERROR",
    "VFS_ERROR",
    "SHELL_ERROR",
    "RUNTIME_ERROR",
    "WASM_ERROR",
]


@dataclass(frozen=True)
class SandboxError(Exception):
    code: ErrorCode
    message: str
    cause: Exception | None = None

    def __str__(self) -> str:
        return f"[{self.code}] {self.message}"


def internal(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("INTERNAL", message, cause)


def invalid_argument(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("INVALID_ARGUMENT", message, cause)


def not_found(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("NOT_FOUND", message, cause)


def already_exists(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("ALREADY_EXISTS", message, cause)


def permission_denied(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("PERMISSION_DENIED", message, cause)


def resource_exhausted(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("RESOURCE_EXHAUSTED", message, cause)


def unavailable(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("UNAVAILABLE", message, cause)


def timeout(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("TIMEOUT", message, cause)


def cancelled(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("CANCELLED", message, cause)


def invalid_state(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("INVALID_STATE", message, cause)


def unsupported(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("UNSUPPORTED", message, cause)


def policy_denied(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("POLICY_DENIED", message, cause)


def promotion_failed(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("PROMOTION_FAILED", message, cause)


def snapshot_failed(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("SNAPSHOT_FAILED", message, cause)


def vm_boot_failed(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("VM_BOOT_FAILED", message, cause)


def network_error(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("NETWORK_ERROR", message, cause)


def cas_error(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("CAS_ERROR", message, cause)


def vfs_error(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("VFS_ERROR", message, cause)


def shell_error(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("SHELL_ERROR", message, cause)


def runtime_error(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("RUNTIME_ERROR", message, cause)


def wasm_error(message: str, cause: Exception | None = None) -> SandboxError:
    return SandboxError("WASM_ERROR", message, cause)


def is_sandbox_error(error: object) -> TypeGuard[SandboxError]:
    return isinstance(error, SandboxError)


T = TypeVar("T")
Result: TypeAlias = T | SandboxError
