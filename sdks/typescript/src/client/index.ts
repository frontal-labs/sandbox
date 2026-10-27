import type { Result, SandboxError } from "../errors/index.js"
import { createSandboxError, isSandboxError, SandboxErrorCodes } from "../errors/index.js"
import type { ErrorCode } from "../errors/index.js"
import * as z from "zod"
import {
  SessionConfigSchema,
  type ExecRequest,
  type ExecResult,
  type SessionConfig,
  type SessionState,
  type Snapshot,
  wireSchemas,
  wireResponseSchemas,
} from "../types/index.js"

export interface ClientConfig {
  baseUrl: string
  apiKey?: string | undefined
}

export class Client {
  private baseUrlValue: string
  private apiKey: string | undefined

  constructor(config: ClientConfig) {
    this.baseUrlValue = config.baseUrl.replace(/\/$/, "")
    this.apiKey = config.apiKey
  }

  get baseUrl(): string {
    return this.baseUrlValue
  }

  static fromEnv(): Client {
    const baseUrl = process.env["SANDBOX_BASE_URL"] ?? "http://localhost:8080"
    const apiKey = process.env["SANDBOX_API_KEY"]
    const config: ClientConfig = { baseUrl }
    if (apiKey !== undefined) {
      config.apiKey = apiKey
    }
    return new Client(config)
  }

  private headers(contentType: boolean): Record<string, string> {
    const headers: Record<string, string> = {}
    if (contentType) {
      headers["Content-Type"] = "application/json"
    }
    if (this.apiKey !== undefined) {
      headers["Authorization"] = `Bearer ${this.apiKey}`
    }
    return headers
  }

private async request<T>(
    path: string,
    options: RequestInit = {},
    responseSchema?: z.ZodType<T>,
  ): Promise<Result<T>> {
    const url = `${this.baseUrlValue}${path}`
    try {
      const response = await fetch(url, {
        ...options,
        headers: { ...this.headers(options.body !== undefined), ...(options.headers as Record<string, string>) },
      })

      if (!response.ok) {
        return this.errorFromResponse(response)
      }

      if (response.status === 204) {
        return undefined as T
      }

      const data = await response.json()

      if (responseSchema) {
        const parseResult = responseSchema.safeParse(data)
        if (!parseResult.success) {
          return SandboxErrorCodes.internal(
            `Response validation failed: ${parseResult.error.message}`,
          )
        }
        return parseResult.data
      }

      return data as T
    } catch (error) {
      if (isSandboxError(error)) {
        return error
      }
      return SandboxErrorCodes.networkError(
        error instanceof Error ? error.message : "Network error",
        error instanceof Error ? error : undefined,
      )
    }
  }

  /// Creates a session. Returns the new session's ID.
  async createSession(config: SessionConfig): Promise<Result<string>> {
    const parseResult = SessionConfigSchema.safeParse(config)
    if (!parseResult.success) {
      return SandboxErrorCodes.invalidArgument(
        `Invalid session config: ${parseResult.error.message}`,
      )
    }

    const created = await this.request(
      "/v1/sessions",
      { method: "POST", body: JSON.stringify(wireSchemas.SessionConfig.parse(parseResult.data)) },
      wireResponseSchemas.CreateSessionResponse,
    )
    if (isSandboxError(created)) {
      return created
    }
    return created.sessionId
  }

  async getSession(sessionId: string): Promise<Result<SessionState>> {
    return this.request<SessionState>(
      `/v1/sessions/${sessionId}`,
      {},
      wireResponseSchemas.SessionState,
    )
  }

  async destroySession(sessionId: string): Promise<Result<void>> {
    return this.request<void>(`/v1/sessions/${sessionId}`, {
      method: "DELETE",
    })
  }

  async exec(sessionId: string, request: ExecRequest): Promise<Result<ExecResult>> {
    const parseResult = wireSchemas.ExecRequest.safeParse(request)
    if (!parseResult.success) {
      return SandboxErrorCodes.invalidArgument(
        `Invalid exec request: ${parseResult.error.message}`,
      )
    }

    const wire = parseResult.data
    return this.request<ExecResult>(
      `/v1/sessions/${sessionId}/exec`,
      { method: "POST", body: JSON.stringify(wire) },
      wireResponseSchemas.ExecResult,
    )
  }

  /// Streaming exec is not implemented on the server, so this yields one
  /// `UNSUPPORTED` error and does not send a request.
  async *execStream(
    _sessionId: string,
    _request: ExecRequest,
  ): AsyncGenerator<Result<ExecResult>, void, unknown> {
    yield SandboxErrorCodes.unsupported("streaming exec is not implemented yet")
  }

  async snapshot(sessionId: string): Promise<Result<Snapshot>> {
    const taken = await this.request(
      `/v1/sessions/${sessionId}/snapshot`,
      { method: "POST" },
      wireResponseSchemas.TakeSnapshotResponse,
    )
    if (isSandboxError(taken)) {
      return taken
    }
    return this.getSnapshot(sessionId, taken.snapshotId)
  }

  /** Puts the session to sleep and returns the id of the snapshot it wakes from. */
  async sleep(sessionId: string): Promise<Result<string>> {
    const taken = await this.request(
      `/v1/sessions/${sessionId}/sleep`,
      { method: "POST" },
      wireResponseSchemas.TakeSnapshotResponse,
    )
    if (isSandboxError(taken)) {
      return taken
    }
    return taken.snapshotId
  }

  /** Wakes a sleeping session. Its files come back; shell state starts fresh. */
  async wake(sessionId: string): Promise<Result<SessionState>> {
    return this.request<SessionState>(
      `/v1/sessions/${sessionId}/wake`,
      { method: "POST" },
      wireResponseSchemas.SessionState,
    )
  }

  async listSnapshots(sessionId: string): Promise<Result<Snapshot[]>> {
    return this.request<Snapshot[]>(
      `/v1/sessions/${sessionId}/snapshots`,
      {},
      z.array(wireResponseSchemas.Snapshot),
    )
  }

  async getSnapshot(sessionId: string, snapshotId: string): Promise<Result<Snapshot>> {
    return this.request<Snapshot>(
      `/v1/sessions/${sessionId}/snapshots/${snapshotId}`,
      {},
      wireResponseSchemas.Snapshot,
    )
  }

  /// Starts `count` sessions from a snapshot. Returns their IDs.
  async fork(snapshotId: string, count: number): Promise<Result<string[]>> {
    const forked = await this.request(
      `/v1/snapshots/${snapshotId}/fork`,
      { method: "POST", body: JSON.stringify({ count }) },
      wireResponseSchemas.ForkResponse,
    )
    if (isSandboxError(forked)) {
      return forked
    }
    return forked.sessionIds
  }

  /// Reads a file from the session's filesystem. Returns the raw bytes.
  async readFile(sessionId: string, path: string): Promise<Result<Uint8Array>> {
    const url = `/v1/sessions/${sessionId}/fs/${encodeFsPath(path)}`
    const response = await this.fetch(url, { method: "GET" })
    if (isSandboxError(response)) {
      return response
    }
    return new Uint8Array(await response.arrayBuffer())
  }

  /// Writes a file, creating it or replacing it.
  async writeFile(
    sessionId: string,
    path: string,
    bytes: Uint8Array,
  ): Promise<Result<void>> {
    const url = `/v1/sessions/${sessionId}/fs/${encodeFsPath(path)}`
    const response = await this.fetch(url, {
      method: "PUT",
      headers: { "Content-Type": "application/octet-stream" },
      body: bytes as BodyInit,
    })
    return isSandboxError(response) ? response : (undefined as void)
  }

  /// Deletes a file or an empty directory.
  async deleteFile(sessionId: string, path: string): Promise<Result<void>> {
    const url = `/v1/sessions/${sessionId}/fs/${encodeFsPath(path)}`
    const response = await this.fetch(url, { method: "DELETE" })
    return isSandboxError(response) ? response : (undefined as void)
  }

  private async fetch(url: string, init: RequestInit = {}): Promise<Result<Response>> {
    const fullUrl = `${this.baseUrlValue}${url}`
    try {
      const response = await fetch(fullUrl, {
        ...init,
        headers: {
          ...this.headers(init.body !== undefined),
          ...(init.headers as Record<string, string>),
        },
      })
      if (!response.ok) {
        return this.errorFromResponse(response)
      }
      return response
    } catch (error) {
      if (isSandboxError(error)) {
        return error
      }
      return SandboxErrorCodes.networkError(
        error instanceof Error ? error.message : "Network error",
        error instanceof Error ? error : undefined,
      )
    }
  }

  private async errorFromResponse(response: Response): Promise<SandboxError> {
    const body = (await response.json().catch(() => ({}))) as {
      code?: string
      message?: string
    }
    const message =
      typeof body.message === "string"
        ? body.message
        : `HTTP ${response.status}: ${response.statusText}`
    if (typeof body.code === "string") {
      return createSandboxError(body.code as ErrorCode, message)
    }
    const code: ErrorCode =
      response.status === 401
        ? "PERMISSION_DENIED"
        : response.status === 404
          ? "NOT_FOUND"
          : response.status === 400
            ? "INVALID_ARGUMENT"
            : response.status === 501
              ? "UNSUPPORTED"
              : "INTERNAL"
    return createSandboxError(code, message)
  }
}

/// Percent-encodes one path segment for the filesystem API.
function encodeFsPath(path: string): string {
  return path
    .trimStart()
    .split("/")
    .map((segment) => encodeURIComponent(segment))
    .join("/")
}
