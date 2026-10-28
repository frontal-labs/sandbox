import { afterEach, describe, expect, it, vi } from "vitest"
import { Client } from "../src/client/index.js"
import { isSandboxError } from "../src/errors/index.js"

const realFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = realFetch
  vi.restoreAllMocks()
})

describe("filesystem paths", () => {
  it("refuses dot segments before any request is sent", async () => {
    const fetchMock = vi.fn(async () => new Response("{}", { status: 200 }))
    globalThis.fetch = fetchMock as unknown as typeof fetch
    const client = new Client({ baseUrl: "http://sandbox.test" })

    const result = await client.readFile("01SESSION", "a/../../snapshots")

    expect(isSandboxError(result)).toBe(true)
    expect(isSandboxError(result) && result.code).toBe("INVALID_ARGUMENT")
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it("encodes each segment and the session ID", async () => {
    const fetchMock = vi.fn(async () => new Response(new Uint8Array([1]), { status: 200 }))
    globalThis.fetch = fetchMock as unknown as typeof fetch
    const client = new Client({ baseUrl: "http://sandbox.test" })

    await client.readFile("01S/ESSION", "dir name/file.txt")

    const [url] = fetchMock.mock.calls[0] as unknown as [string]
    expect(url).toBe("http://sandbox.test/v1/sessions/01S%2FESSION/fs/dir%20name/file.txt")
  })
})
