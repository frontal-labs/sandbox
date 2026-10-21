import { afterEach, describe, expect, it, vi } from "vitest"
import { Client } from "../src/client/index.js"
import { isSandboxError } from "../src/errors/index.js"
import { ExecResultSchema, SessionStateSchema } from "../src/types/index.js"

// These tests check the wire format against the server's JSON shapes, using a
// stubbed fetch. The integration test at the bottom runs against a live server.

const jsonResponse = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

afterEach(() => {
  vi.unstubAllGlobals()
})

describe("wire format", () => {
  it("parses a session state with snake_case keys and RFC 3339 timestamps", () => {
    const parsed = SessionStateSchema.parse({
      id: "01ARZ3NDEKTSV4RRFFQ69G5FAV",
      tenant_id: "00000000000000000000000000",
      tier: "isolate",
      status: "running",
      config_hash: "0".repeat(64),
      created_at: "2026-10-15T12:00:00.000Z",
      updated_at: "2026-10-15T12:00:00.000Z",
      labels: {},
    })
    expect(parsed.tenant_id).toBe("00000000000000000000000000")
  })

  it("turns the server's byte arrays into Uint8Array", () => {
    const parsed = ExecResultSchema.parse({ exit_code: 0, stdout: [104, 105], stderr: [], cpu_ms: 0, wall_ms: 1 })
    expect(parsed.stdout).toBeInstanceOf(Uint8Array)
    expect(new TextDecoder().decode(parsed.stdout)).toBe("hi")
  })

  it("creates a session from the server's { session_id } body and sends snake_case", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV" }, 201))
    vi.stubGlobal("fetch", fetchMock)
    const client = new Client({ baseUrl: "http://server.test", apiKey: "k" })

    const result = await client.createSession({
      tenant_id: "00000000000000000000000000",
      tier_preference: "isolate",
      limits: {},
      network: { default_deny: true, rules: [] },
      env: {},
      cwd: "/work",
      labels: {},
    })

    expect(result).toBe("01ARZ3NDEKTSV4RRFFQ69G5FAV")
    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    const sent = JSON.parse(init.body as string)
    expect(sent.tenant_id).toBe("00000000000000000000000000")
    expect(sent.tier_preference).toBe("isolate")
    expect(sent.network.default_deny).toBe(true)
    expect((init.headers as Record<string, string>)["Authorization"]).toBe("Bearer k")
  })

  it("sends stdin as an integer array and returns stdout as bytes", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ exit_code: 0, stdout: [111, 107], stderr: [], cpu_ms: 0, wall_ms: 2 }),
    )
    vi.stubGlobal("fetch", fetchMock)
    const client = new Client({ baseUrl: "http://server.test" })

    const result = await client.exec("01ARZ3NDEKTSV4RRFFQ69G5FAV", {
      argv: ["cat"],
      env: {},
      stdin: new Uint8Array([1, 2]),
      stream: false,
    })

    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(JSON.parse(init.body as string).stdin).toEqual([1, 2])
    if (isSandboxError(result)) throw result
    expect(new TextDecoder().decode(result.stdout)).toBe("ok")
  })

  it("carries the server's error code through", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => jsonResponse({ code: "NOT_FOUND", message: "session not found" }, 404)))
    const client = new Client({ baseUrl: "http://server.test" })

    const result = await client.getSession("01ARZ3NDEKTSV4RRFFQ69G5FAV")
    expect(isSandboxError(result)).toBe(true)
    if (isSandboxError(result)) {
      expect(result.code).toBe("NOT_FOUND")
    }
  })

  it("refuses streaming exec without sending a request", async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal("fetch", fetchMock)
    const client = new Client({ baseUrl: "http://server.test" })

    const chunks = []
    for await (const chunk of client.execStream("01ARZ3NDEKTSV4RRFFQ69G5FAV", { argv: ["echo"], env: {}, stream: true })) {
      chunks.push(chunk)
    }
    expect(fetchMock).not.toHaveBeenCalled()
    expect(chunks).toHaveLength(1)
    expect(isSandboxError(chunks[0])).toBe(true)
  })
})

// Runs only when a live server is configured, for example:
//   SANDBOX_BASE_URL=http://127.0.0.1:8080 SANDBOX_API_KEY=... npx vitest run tests/wire.test.ts
const live = process.env["SANDBOX_BASE_URL"] !== undefined && process.env["SANDBOX_API_KEY"] !== undefined

describe.skipIf(!live)("live server", () => {
  it("creates a session, runs a command, and destroys it", async () => {
    const client = Client.fromEnv()
    const id = await client.createSession({
      tenant_id: "00000000000000000000000000",
      tier_preference: "isolate",
      limits: {},
      network: { default_deny: true, rules: [] },
      env: {},
      cwd: "/work",
      labels: {},
    })
    if (isSandboxError(id)) throw id

    const out = await client.exec(id, { argv: ["echo", "hello"], env: {}, stream: false })
    if (isSandboxError(out)) throw out
    expect(out.exit_code).toBe(0)
    expect(new TextDecoder().decode(out.stdout).trim()).toBe("hello")

    const destroyed = await client.destroySession(id)
    expect(isSandboxError(destroyed)).toBe(false)
  })

  it("sleeps a session, wakes on demand, and resumes work", async () => {
    const client = Client.fromEnv()
    const id = await client.createSession({
      tenant_id: "00000000000000000000000000",
      tier_preference: "isolate",
      limits: {},
      network: { default_deny: true, rules: [] },
      env: {},
      cwd: "/work",
      labels: {},
    })
    if (isSandboxError(id)) throw id

    const snapshotId = await client.sleep(id)
    if (isSandboxError(snapshotId)) throw snapshotId
    expect(snapshotId.length).toBeGreaterThan(0)
    const asleep = await client.getSession(id)
    if (isSandboxError(asleep)) throw asleep
    expect(asleep.status).toBe("sleeping")

    const woken = await client.wake(id)
    if (isSandboxError(woken)) throw woken
    expect(woken.status).toBe("running")

    const out = await client.exec(id, { argv: ["echo", "awake"], env: {}, stream: false })
    if (isSandboxError(out)) throw out
    expect(new TextDecoder().decode(out.stdout).trim()).toBe("awake")

    await client.destroySession(id)
  })
})
