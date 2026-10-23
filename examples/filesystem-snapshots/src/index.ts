import {
  Client,
  isSandboxError,
  type SessionConfig,
  type Snapshot,
} from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates filesystem snapshot management:
 * create session -> write data -> snapshot -> list snapshots -> fork from snapshot -> verify
 */
async function main(): Promise<void> {
  // Create client from environment variables (SANDBOX_BASE_URL, SANDBOX_API_KEY)
  const client = Client.fromEnv()

  // Session configuration
  const sessionConfig: SessionConfig = {
    tenant_id: "fs-snapshots",
    tier_preference: "auto",
    cwd: "/work",
    limits: {},
    network: { default_deny: true, rules: [] },
    env: {},
    labels: {},
  }

  // Create a new sandbox session
  const sessionResult = await client.createSession(sessionConfig)
  if (isSandboxError(sessionResult)) {
    throw sessionResult
  }
  const sessionId = sessionResult

  // Write test data to the sandbox filesystem
  const writeResult = await client.exec(sessionId, {
    argv: ["sh", "-c", "echo data > /work/data.txt"],
    env: {},
    stream: false,
  })
  if (isSandboxError(writeResult)) {
    throw writeResult
  }

  // Create a snapshot capturing the current filesystem state
  const snapshotResult = await client.snapshot(sessionId)
  if (isSandboxError(snapshotResult)) {
    throw snapshotResult
  }
  const snapshot: Snapshot = snapshotResult
  console.log("Created snapshot:", snapshot.id)

  // List all snapshots for this session
  const listResult = await client.listSnapshots(sessionId)
  if (isSandboxError(listResult)) {
    throw listResult
  }
  console.log(
    "All snapshots:",
    listResult.map((s) => s.id),
  )

  // Fork a new session from the snapshot
  const forkResult = await client.fork(snapshot.id, 1)
  if (isSandboxError(forkResult)) {
    throw forkResult
  }
  const [forkedId] = forkResult

  // Verify the forked session can read the data from the snapshot
  const readResult = await client.exec(forkedId, {
    argv: ["cat", "/work/data.txt"],
    env: {},
    stream: false,
  })
  if (isSandboxError(readResult)) {
    throw readResult
  }
  console.log("Forked read:", new TextDecoder().decode(readResult.stdout).trim())

  // Clean up forked session
  await client.destroySession(forkedId)

  // Clean up original session
  await client.destroySession(sessionId)
  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
