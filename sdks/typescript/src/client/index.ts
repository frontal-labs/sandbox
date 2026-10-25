import type { Result, SandboxError } from "../errors/index.js"
import { createSandboxError, isSandboxError, SandboxErrorCodes } from "../errors/index.js"
import type { ErrorCode } from "../errors/index.js"
import * as z from "zod"
import {
  CreateSessionResponseSchema,
  ExecRequestSchema,
  ExecResultSchema,
  ForkResponseSchema,
  SessionConfigSchema,
  SessionStateSchema,
  SnapshotSchema,
  TakeSnapshotResponseSchema,
  type ExecRequest,
  type ExecResult,
  type SessionConfig,
  type SessionState,
  type Snapshot,
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
        return errorFromResponse(response)
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
        return parseResult.data as T
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
      { method: "POST", body: JSON.stringify(parseResult.data) },
      CreateSessionResponseSchema,
    )
    if (isSandboxError(created)) {
      return created
    }
    return created.session_id
  }

  async getSession(sessionId: string): Promise<Result<SessionState>> {
    return this.request<SessionState>(`/v1/sessions/${sessionId}`, {}, SessionStateSchema)
  }

  async destroySession(sessionId: string): Promise<Result<void>> {
    return this.request<void>(`/v1/sessions/${sessionId}`, {
      method: "DELETE",
    })
  }

  async exec(sessionId: string, request: ExecRequest): Promise<Result<ExecResult>> {
    const parseResult = ExecRequestSchema.safeParse(request)
    if (!parseResult.success) {
      return SandboxErrorCodes.invalidArgument(
        `Invalid exec request: ${parseResult.error.message}`,
      )
    }

    const { stdin, ...rest } = parseResult.data
    const wire = { ...rest, stdin: stdin === undefined ? undefined : Array.from(stdin) }
    return this.request<ExecResult>(
      `/v1/sessions/${sessionId}/exec`,
      { method: "POST", body: JSON.stringify(wire) },
      ExecResultSchema,
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
      TakeSnapshotResponseSchema,
    )
    if (isSandboxError(taken)) {
      return taken
    }
    return this.getSnapshot(sessionId, taken.snapshot_id)
  }

  /** Puts the session to sleep and returns the id of the snapshot it wakes from. */
  async sleep(sessionId: string): Promise<Result<string>> {
    const taken = await this.request(
      `/v1/sessions/${sessionId}/sleep`,
      { method: "POST" },
      TakeSnapshotResponseSchema,
    )
    if (isSandboxError(taken)) {
      return taken
    }
    return taken.snapshot_id
  }

  /** Wakes a sleeping session. Its files come back; shell state starts fresh. */
  async wake(sessionId: string): Promise<Result<SessionState>> {
    return this.request<SessionState>(
      `/v1/sessions/${sessionId}/wake`,
      { method: "POST" },
      SessionStateSchema,
    )
  }

  async listSnapshots(sessionId: string): Promise<Result<Snapshot[]>> {
    return this.request<Snapshot[]>(
      `/v1/sessions/${sessionId}/snapshots`,
      {},
      z.array(SnapshotSchema),
    )
  }

  async getSnapshot(sessionId: string, snapshotId: string): Promise<Result<Snapshot>> {
    return this.request<Snapshot>(
      `/v1/sessions/${sessionId}/snapshots/${snapshotId}`,
      {},
      SnapshotSchema,
    )
  }

  /// Starts `count` sessions from a snapshot. Returns their IDs.
  async fork(snapshotId: string, count: number): Promise<Result<string[]>> {
    const forked = await this.request(
      `/v1/snapshots/${snapshotId}/fork`,
      { method: "POST", body: JSON.stringify({ count }) },
      ForkResponseSchema,
    )
    if (isSandboxError(forked)) {
      return forked
    }
    return forked.session_ids
  }
}

/// Maps a non-2xx response to the error the server sent. The server's error
/// body is `{ code, message }`. A body without it is mapped from the status.
async function errorFromResponse(response: Response): Promise<SandboxError> {
  const body = (await response.json().catch(() => ({}))) as { code?: unknown; message?: unknown }
  const message =
    typeof body.message === "string" ? body.message : `HTTP ${response.status}: ${response.statusText}`
  if (typeof body.code === "string") {
    return createSandboxError(body.code as ErrorCode, message)
  }
  const code: ErrorCode =
    response.status === 401 ? "PERMISSION_DENIED"
    : response.status === 404 ? "NOT_FOUND"
    : response.status === 400 ? "INVALID_ARGUMENT"
    : response.status === 501 ? "UNSUPPORTED"
    : "INTERNAL"
  return createSandboxError(code, message)
}
