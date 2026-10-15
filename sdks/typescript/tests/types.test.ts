import { describe, expect, it } from "vitest"
import { isSandboxError, SandboxError, SandboxErrorCodes } from "../src/errors/index.js"
import {
  type ExecRequest,
  type ExecResult,
  generateId,
  type NetworkPolicy,
  nilId,
  type ResourceLimits,
  type SessionConfig,
  SessionStatus,
  type Snapshot,
  Tier,
  TierPreference,
} from "../src/types/index.js"

describe("types", () => {
  it("should create session config with builder", () => {
    const config: SessionConfig = {
      tenant_id: "tenant-123",
      tier_preference: "auto",
      limits: {},
      network: { default_deny: true, rules: [] },
      env: {},
      cwd: "/work",
      labels: {},
    }

    expect(config.tenant_id).toBe("tenant-123")
    expect(config.tier_preference).toBe("auto")
    expect(config.cwd).toBe("/work")
  })

  it("should create resource limits", () => {
    const limits: ResourceLimits = {
      memory_bytes: 1024 * 1024 * 1024,
      cpu_ms_per_sec: 500,
    }

    expect(limits.memory_bytes).toBe(1024 * 1024 * 1024)
    expect(limits.cpu_ms_per_sec).toBe(500)
  })

  it("should create network policy", () => {
    const policy: NetworkPolicy = {
      default_deny: false,
      rules: [],
    }

    expect(policy.default_deny).toBe(false)
    expect(policy.rules).toEqual([])
  })

  it("should generate unique IDs", () => {
    const id1 = generateId()
    const id2 = generateId()
    expect(id1.length).toBe(26)
    expect(id1).not.toBe(id2)
  })

  it("should create nil ID", () => {
    const nil = nilId()
    expect(nil).toBe("00000000000000000000000000")
  })
})

describe("errors", () => {
  it("should create sandbox errors", () => {
    const err = SandboxErrorCodes.internal("test error")
    expect(err.code).toBe("INTERNAL")
    expect(err.message).toBe("[INTERNAL] test error")
  })

  it("should create invalid state error", () => {
    const err = SandboxErrorCodes.invalidState("no session")
    expect(err.code).toBe("INVALID_STATE")
    expect(isSandboxError(err)).toBe(true)
  })

  it("should not match regular errors", () => {
    expect(isSandboxError(new Error("test"))).toBe(false)
  })
})

describe("exec request", () => {
  it("should create exec request", () => {
    const req: ExecRequest = {
      argv: ["echo", "hello"],
      env: { FOO: "bar" },
      stream: true,
    }

    expect(req.argv).toEqual(["echo", "hello"])
    expect(req.env).toEqual({ FOO: "bar" })
    expect(req.stream).toBe(true)
  })
})

describe("exec result", () => {
  it("should create exec result", () => {
    const result: ExecResult = {
      exit_code: 0,
      stdout: new TextEncoder().encode("hello"),
      stderr: new TextEncoder().encode(""),
      cpu_ms: 10,
      wall_ms: 20,
    }

    expect(result.exit_code).toBe(0)
    expect(result.cpu_ms).toBe(10)
  })
})

describe("snapshot", () => {
  it("should create snapshot", () => {
    const snap: Snapshot = {
      id: "01ARZ3NDEKTSV4RRFFQ69G5FAV",
      parent: undefined,
      vfs_root: "hash123",
      env: {},
      cwd: "/work",
      tier: "isolate",
      runtime_blob: undefined,
      created_at: "2024-01-01T00:00:00Z",
      labels: {},
    }

    expect(snap.id).toBe("01ARZ3NDEKTSV4RRFFQ69G5FAV")
    expect(snap.tier).toBe("isolate")
  })
})
