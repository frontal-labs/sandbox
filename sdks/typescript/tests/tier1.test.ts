import { afterEach, describe, expect, it, vi } from "vitest"
import { Tier1Client } from "../src/client/tier1.js"

const realFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = realFetch
  vi.restoreAllMocks()
})

function reply(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

describe("Tier1Client", () => {
  it("posts an undo with the step count and the API key", async () => {
    const fetchMock = vi.fn(async () =>
      reply(200, { root: "ab", checkpoints: { policy: "per_exec", count: 2, retain: 50, bytes: 5 } }),
    )
    globalThis.fetch = fetchMock as unknown as typeof fetch
    const client = new Tier1Client({ baseUrl: "http://sandbox.test/", apiKey: "key" })

    const result = await client.undo("01SESSION", 2)

    expect(result.checkpoints.count).toBe(2)
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(url).toBe("http://sandbox.test/v1/sessions/01SESSION/undo")
    expect(init.method).toBe("POST")
    expect(JSON.parse(String(init.body))).toEqual({ steps: 2 })
    expect((init.headers as Record<string, string>)["Authorization"]).toBe("Bearer key")
  })

  it("maps a server error to a SandboxError with its code", async () => {
    globalThis.fetch = vi.fn(async () =>
      reply(400, { code: "INVALID_ARGUMENT", message: "only 0 checkpoint(s) to undo to" }),
    ) as unknown as typeof fetch
    const client = new Tier1Client({ baseUrl: "http://sandbox.test" })

    const failure = client.undo("01SESSION")
    await expect(failure).rejects.toMatchObject({ code: "INVALID_ARGUMENT" })
    await expect(failure).rejects.toThrow("only 0 checkpoint(s) to undo to")
  })

  it("returns the children of a fork", async () => {
    globalThis.fetch = vi.fn(async () =>
      reply(200, {
        children: [{ session_id: "01CHILD", exit_code: 0, error: null, changed_files: 1 }],
      }),
    ) as unknown as typeof fetch
    const client = new Tier1Client({ baseUrl: "http://sandbox.test" })

    const children = await client.forkCommand("01SESSION", 1, ["touch", "/r"])

    expect(children).toEqual([{ sessionId: "01CHILD", exitCode: 0, error: null, changedFiles: 1 }])
  })
})
