import { Client } from "../client/index.js"
import type { Result, SandboxError } from "../errors/index.js"
import { isSandboxError, SandboxErrorCodes } from "../errors/index.js"
import {
  SessionConfigSchema,
  type ExecResult,
  type SessionConfig,
  type Snapshot,
} from "../types/index.js"

export interface EmbeddedSandboxConfig extends SessionConfig {
  serverPath?: string
}

export class EmbeddedSandbox {
  private client: Client
  private sessionId: string | undefined

  private constructor(client: Client, sessionId: string) {
    this.client = client
    this.sessionId = sessionId
  }

  static async new(config: EmbeddedSandboxConfig): Promise<Result<EmbeddedSandbox>> {
    const { serverPath = "sandbox-server", ...sessionConfig } = config

    const parseResult = SessionConfigSchema.safeParse(sessionConfig)
    if (!parseResult.success) {
      return SandboxErrorCodes.invalidArgument(
        `Invalid session config: ${parseResult.error.message}`,
      )
    }

    try {
      const client = Client.fromEnv()

      if (client.baseUrl === "http://localhost:8080") {
        try {
          const controller = new AbortController()
          const timeout = setTimeout(() => controller.abort(), 2000)
          await fetch("http://localhost:8080/health", { signal: controller.signal })
          clearTimeout(timeout)
        } catch {
          return SandboxErrorCodes.unavailable(
            "No sandbox server running. Start 'sandbox-server' or set SANDBOX_BASE_URL.",
          )
        }
      }

      const sessionResult = await client.createSession(parseResult.data)
      if (isSandboxError(sessionResult)) {
        return sessionResult
      }

      return new EmbeddedSandbox(client, sessionResult)
    } catch (error) {
      return SandboxErrorCodes.internal(
        error instanceof Error ? error.message : "Failed to create embedded sandbox",
      )
    }
  }

  async exec(command: string): Promise<Result<ExecResult>> {
    if (!this.sessionId) {
      return SandboxErrorCodes.invalidState("No active session")
    }

    // The isolate shell has no `sh`, so the command is split into argv, as the
    // Rust SDK does. Quoting is not interpreted.
    const argv = command.split(/\s+/).filter((word) => word.length > 0)
    if (argv.length === 0) {
      return SandboxErrorCodes.invalidArgument("command is empty")
    }
    return this.client.exec(this.sessionId, { argv, env: {}, stream: false })
  }

  async snapshot(): Promise<Result<Snapshot>> {
    if (!this.sessionId) {
      return SandboxErrorCodes.invalidState("No active session")
    }

    return this.client.snapshot(this.sessionId)
  }

  async destroy(): Promise<Result<void>> {
    if (!this.sessionId) {
      return SandboxErrorCodes.invalidState("No active session")
    }

    const result = await this.client.destroySession(this.sessionId)
    this.sessionId = undefined
    return result
  }

  getSessionId(): string | undefined {
    return this.sessionId
  }
}