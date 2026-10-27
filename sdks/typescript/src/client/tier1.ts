// Tier 1 session tools: checkpoints, undo, the environment, and fork with a command.
// Kept apart from `Client` so each call is one request to the /v1 API. The server
// routes are documented in docs/tier1-agent-interfaces.md.

import { createSandboxError } from "../errors/index.js"
import type { ErrorCode } from "../errors/index.js"

export interface Tier1Config {
  baseUrl: string
  apiKey?: string | undefined
}

export interface CheckpointUsage {
  policy: "per_exec" | "manual"
  count: number
  retain: number
  /** Logical bytes of the pinned roots. */
  bytes: number
}

export interface CheckpointResponse {
  root: string
  checkpoints: CheckpointUsage
}

export interface ForkedChild {
  session_id: string
  /** The exit code, or null when the command could not run. */
  exit_code: number | null
  /** Why the command could not run, when it did not. */
  error: string | null
  changed_files: number
}

export class Tier1Client {
  private readonly baseUrlValue: string
  private readonly apiKey: string | undefined

  constructor(config: Tier1Config) {
    this.baseUrlValue = config.baseUrl.replace(/\/$/, "")
    this.apiKey = config.apiKey
  }

  /** Pins the session's current state as a checkpoint. */
  checkpoint(session: string): Promise<CheckpointResponse> {
    return this.call<CheckpointResponse>("POST", `/v1/sessions/${session}/checkpoint`)
  }

  /** Restores the filesystem from `steps` checkpoints ago. */
  undo(session: string, steps = 1): Promise<CheckpointResponse> {
    return this.call<CheckpointResponse>("POST", `/v1/sessions/${session}/undo`, { steps })
  }

  /** The session's environment, as /etc/agent/env.json. */
  info(session: string): Promise<Record<string, unknown>> {
    return this.call<Record<string, unknown>>("GET", `/v1/sessions/${session}/info`)
  }

  /** Forks the session into `count` children and runs `argv` in each. */
  async forkCommand(session: string, count: number, argv: string[]): Promise<ForkedChild[]> {
    const body = await this.call<{ children: ForkedChild[] }>(
      "POST",
      `/v1/sessions/${session}/fork`,
      { count, argv },
    )
    return body.children
  }

  private async call<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = {}
    if (body !== undefined) headers["Content-Type"] = "application/json"
    if (this.apiKey !== undefined) headers["Authorization"] = `Bearer ${this.apiKey}`
    const init: RequestInit = { method, headers }
    if (body !== undefined) init.body = JSON.stringify(body)
    const response = await fetch(`${this.baseUrlValue}${path}`, init)
    if (!response.ok) {
      let code: ErrorCode = "INTERNAL"
      let message = response.statusText
      try {
        const error = (await response.json()) as { code?: ErrorCode; message?: string }
        code = error.code ?? code
        message = error.message ?? message
      } catch {
        // The body was not JSON. The status text stands in for the message.
      }
      throw createSandboxError(code, message)
    }
    return (await response.json()) as T
  }
}
