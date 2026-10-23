import {
  Client,
  isSandboxError,
  type SessionConfig,
  type Snapshot,
} from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates forking a snapshot into multiple parallel worker sessions.
 * Each forked session gets its own copy of the filesystem state at snapshot time.
 */
async function main(): Promise<void> {
  // Create client from environment variables (SANDBOX_BASE_URL, SANDBOX_API_KEY)
  const client = Client.fromEnv()

  // Session configuration for the parent session
  const sessionConfig: SessionConfig = {
    tenant_id: "fork-parallel",
    tier_preference: "auto",
    cwd: "/work",
    limits: {},
    network: { default_deny: true, rules: [] },
    env: {},
    labels: {},
  }

  // Create parent session
  const sessionResult = await client.createSession(sessionConfig)
  if (isSandboxError(sessionResult)) {
    throw sessionResult
  }
  const sessionId = sessionResult

  // Create a snapshot of the parent session's filesystem state
  const snapshotResult = await client.snapshot(sessionId)
  if (isSandboxError(snapshotResult)) {
    throw snapshotResult
  }
  const snapshot: Snapshot = snapshotResult

  // Fork the snapshot into 3 parallel worker sessions
  const forkResult = await client.fork(snapshot.id, 3)
  if (isSandboxError(forkResult)) {
    throw forkResult
  }
  const workerSessions = forkResult

  // Run work in parallel across all forked sessions
  await Promise.all(
    workerSessions.map(async (workerId) => {
      // Each worker executes independently with its own filesystem copy
      const execResult = await client.exec(workerId, {
        argv: ["sh", "-c", "echo worker-$RANDOM"],
        env: {},
        stream: false,
      })
      if (isSandboxError(execResult)) {
        throw execResult
      }

      console.log(`${workerId} -> ${new TextDecoder().decode(execResult.stdout).trim()}`)

      // Clean up each worker session
      await client.destroySession(workerId)
    }),
  )

  // Clean up parent session
  await client.destroySession(sessionId)
  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
