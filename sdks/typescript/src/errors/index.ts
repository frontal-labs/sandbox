export type ErrorCode =
  | "INTERNAL"
  | "INVALID_ARGUMENT"
  | "NOT_FOUND"
  | "ALREADY_EXISTS"
  | "PERMISSION_DENIED"
  | "RESOURCE_EXHAUSTED"
  | "UNAVAILABLE"
  | "TIMEOUT"
  | "CANCELLED"
  | "INVALID_STATE"
  | "UNSUPPORTED"
  | "POLICY_DENIED"
  | "PROMOTION_FAILED"
  | "SNAPSHOT_FAILED"
  | "VM_BOOT_FAILED"
  | "NETWORK_ERROR"
  | "CAS_ERROR"
  | "VFS_ERROR"
  | "SHELL_ERROR"
  | "RUNTIME_ERROR"
  | "WASM_ERROR"

export interface SandboxError extends Error {
  code: ErrorCode
  message: string
  cause?: Error | undefined
}

export function createSandboxError(code: ErrorCode, message: string, cause?: Error): SandboxError {
  const error = new Error(`[${code}] ${message}`) as SandboxError
  error.code = code
  error.cause = cause
  return error
}

export const SandboxErrorCodes = {
  internal: (message: string, cause?: Error) => createSandboxError("INTERNAL", message, cause),
  invalidArgument: (message: string, cause?: Error) =>
    createSandboxError("INVALID_ARGUMENT", message, cause),
  notFound: (message: string, cause?: Error) => createSandboxError("NOT_FOUND", message, cause),
  alreadyExists: (message: string, cause?: Error) =>
    createSandboxError("ALREADY_EXISTS", message, cause),
  permissionDenied: (message: string, cause?: Error) =>
    createSandboxError("PERMISSION_DENIED", message, cause),
  resourceExhausted: (message: string, cause?: Error) =>
    createSandboxError("RESOURCE_EXHAUSTED", message, cause),
  unavailable: (message: string, cause?: Error) =>
    createSandboxError("UNAVAILABLE", message, cause),
  timeout: (message: string, cause?: Error) => createSandboxError("TIMEOUT", message, cause),
  cancelled: (message: string, cause?: Error) => createSandboxError("CANCELLED", message, cause),
  invalidState: (message: string, cause?: Error) =>
    createSandboxError("INVALID_STATE", message, cause),
  unsupported: (message: string, cause?: Error) =>
    createSandboxError("UNSUPPORTED", message, cause),
  policyDenied: (message: string, cause?: Error) =>
    createSandboxError("POLICY_DENIED", message, cause),
  promotionFailed: (message: string, cause?: Error) =>
    createSandboxError("PROMOTION_FAILED", message, cause),
  snapshotFailed: (message: string, cause?: Error) =>
    createSandboxError("SNAPSHOT_FAILED", message, cause),
  vmBootFailed: (message: string, cause?: Error) =>
    createSandboxError("VM_BOOT_FAILED", message, cause),
  networkError: (message: string, cause?: Error) =>
    createSandboxError("NETWORK_ERROR", message, cause),
  casError: (message: string, cause?: Error) => createSandboxError("CAS_ERROR", message, cause),
  vfsError: (message: string, cause?: Error) => createSandboxError("VFS_ERROR", message, cause),
  shellError: (message: string, cause?: Error) => createSandboxError("SHELL_ERROR", message, cause),
  runtimeError: (message: string, cause?: Error) =>
    createSandboxError("RUNTIME_ERROR", message, cause),
  wasmError: (message: string, cause?: Error) => createSandboxError("WASM_ERROR", message, cause),
}

export type Result<T> = T | SandboxError

export function isSandboxError(error: unknown): error is SandboxError {
  return error instanceof Error && "code" in error
}
