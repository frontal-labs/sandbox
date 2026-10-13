import type { Result, SandboxError } from "../errors/index.js"
import { isSandboxError, SandboxErrorCodes } from "../errors/index.js"
import * as z from "zod"
import {
  ExecRequestSchema,
  ExecResultSchema,
  SessionConfigSchema,
  SessionStateSchema,
  SnapshotSchema,
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

  private async request<T>(
    path: string,
    options: RequestInit = {},
    responseSchema?: z.ZodSchema<T>,
  ): Promise<Result<T>> {
    const url = `${this.baseUrlValue}${path}`
    const headers: HeadersInit = {
      "Content-Type": "application/json",
      ...options.headers,
    }

    if (this.apiKey !== undefined) {
      (headers as Record<string, string>)["Authorization"] = `Bearer ${this.apiKey}`
    }

    try {
      const response = await fetch(url, {
        ...options,
        headers,
      })

      if (!response.ok) {
        const errorData = await response.json().catch(() => ({}))
        return SandboxErrorCodes.internal(
          errorData.message ?? `HTTP ${response.status}: ${response.statusText}`,
        )
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

  async createSession(config: SessionConfig): Promise<Result<string>> {
    const parseResult = SessionConfigSchema.safeParse(config)
    if (!parseResult.success) {
      return SandboxErrorCodes.invalidArgument(
        `Invalid session config: ${parseResult.error.message}`,
      )
    }

    return this.request<string>("/v1/sessions", {
      method: "POST",
      body: JSON.stringify(parseResult.data),
    })
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

    return this.request<ExecResult>(
      `/v1/sessions/${sessionId}/exec`,
      {
        method: "POST",
        body: JSON.stringify(parseResult.data),
      },
      ExecResultSchema,
    )
  }

  async *execStream(
    sessionId: string,
    request: ExecRequest,
  ): AsyncGenerator<Result<ExecResult>, void, unknown> {
    const parseResult = ExecRequestSchema.safeParse(request)
    if (!parseResult.success) {
      yield SandboxErrorCodes.invalidArgument(
        `Invalid exec request: ${parseResult.error.message}`,
      )
      return
    }

    const url = `${this.baseUrlValue}/v1/sessions/${sessionId}/exec?stream=true`
    const headers: HeadersInit = {
      "Content-Type": "application/json",
    }

    if (this.apiKey !== undefined) {
      (headers as Record<string, string>)["Authorization"] = `Bearer ${this.apiKey}`
    }

    try {
      const response = await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify(parseResult.data),
      })

      if (!response.ok) {
        const errorData = await response.json().catch(() => ({}))
        yield SandboxErrorCodes.internal(
          errorData.message ?? `HTTP ${response.status}: ${response.statusText}`,
        )
        return
      }

      const reader = response.body?.getReader()
      if (!reader) {
        yield SandboxErrorCodes.internal("No response body")
        return
      }

      const decoder = new TextDecoder()
      let buffer = ""

      while (true) {
        const { done, value } = await reader.read()
        if (done) break

        buffer += decoder.decode(value, { stream: true })
        const lines = buffer.split("\n")
        buffer = lines.pop() ?? ""

        for (const line of lines) {
          if (line.trim()) {
            try {
              const parsed = JSON.parse(line)
              const result = ExecResultSchema.safeParse(parsed)
              if (result.success) {
                yield result.data
              } else {
                yield SandboxErrorCodes.internal(
                  `Failed to parse stream chunk: ${result.error.message}`,
                )
              }
            } catch {
              yield SandboxErrorCodes.internal("Failed to parse stream chunk")
            }
          }
        }
      }

      if (buffer.trim()) {
        try {
          const parsed = JSON.parse(buffer)
          const result = ExecResultSchema.safeParse(parsed)
          if (result.success) {
            yield result.data
          } else {
            yield SandboxErrorCodes.internal(
              `Failed to parse final stream chunk: ${result.error.message}`,
            )
          }
        } catch {
          yield SandboxErrorCodes.internal("Failed to parse final stream chunk")
        }
      }
    } catch (error) {
      if (isSandboxError(error)) {
        yield error
      } else {
        yield SandboxErrorCodes.networkError(
          error instanceof Error ? error.message : "Network error",
          error instanceof Error ? error : undefined,
        )
      }
    }
  }

  async snapshot(sessionId: string): Promise<Result<Snapshot>> {
    return this.request<Snapshot>(
      `/v1/sessions/${sessionId}/snapshot`,
      { method: "POST" },
      SnapshotSchema,
    )
  }

  // Not routed by the server yet (see crates/sandbox-server/src/api.rs).
  async listSnapshots(sessionId: string): Promise<Result<Snapshot[]>> {
    return this.request<Snapshot[]>(
      `/v1/sessions/${sessionId}/snapshots`,
      {},
      z.array(SnapshotSchema),
    )
  }

  // Not routed by the server yet (see crates/sandbox-server/src/api.rs).
  async getSnapshot(sessionId: string, snapshotId: string): Promise<Result<Snapshot>> {
    return this.request<Snapshot>(
      `/v1/sessions/${sessionId}/snapshots/${snapshotId}`,
      {},
      SnapshotSchema,
    )
  }
}