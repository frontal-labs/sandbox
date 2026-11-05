import {
  Client,
  isSandboxError,
  type SessionConfig,
  type Snapshot,
} from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates basic sandbox session lifecycle:
 * create session -> exec commands -> snapshot -> fork -> cleanup
 */
async function main(): Promise<void> {
  // Create client from environment variables (SANDBOX_BASE_URL, SANDBOX_API_KEY)
  const client = Client.fromEnv()

  // Session configuration with all fields explicitly provided
  // (Zod defaults are runtime-only, so we provide all fields for type safety)
  const sessionConfig: SessionConfig = {
    tenantId: "sandbox-basics-example",
    tierPreference: "auto",
    cwd: "/work",
    limits: {},
    network: { defaultDeny: true, rules: [] },
    env: {},
    labels: {},
  }

  // Create a new sandbox session
  const sessionResult = await client.createSession(sessionConfig)
  if (isSandboxError(sessionResult)) {
    throw sessionResult
  }
  const sessionId = sessionResult
  console.log("Created session:", sessionId)

  // Execute a command to show working directory and environment
  const infoResult = await client.exec(sessionId, {
    argv: ["sh", "-c", "pwd && ls -la && echo $PATH"],
    env: {},
    stream: false,
  })
  if (isSandboxError(infoResult)) {
    throw infoResult
  }
  console.log("Exit code:", infoResult.exitCode)
  console.log("Stdout:", new TextDecoder().decode(infoResult.stdout))

  // Write a file to the sandbox filesystem
  const writeResult = await client.exec(sessionId, {
    argv: ["sh", "-c", "echo 'hello from sandbox' > /work/hello.txt && cat /work/hello.txt"],
    env: {},
    stream: false,
  })
  if (isSandboxError(writeResult)) {
    throw writeResult
  }
  console.log("File contents:", new TextDecoder().decode(writeResult.stdout))

  // Create a filesystem snapshot of the session
  const snapshotResult = await client.snapshot(sessionId)
  if (isSandboxError(snapshotResult)) {
    throw snapshotResult
  }
  const snapshot: Snapshot = snapshotResult
  console.log("Created snapshot:", snapshot.id)

  // Fork the snapshot into 2 new sessions
  const forkResult = await client.fork(snapshot.id, 2)
  if (isSandboxError(forkResult)) {
    throw forkResult
  }
  const forkedSessions = forkResult
  console.log("Forked sessions:", forkedSessions)

  // Verify each forked session can read the file
  for (const forkedId of forkedSessions) {
    const readResult = await client.exec(forkedId, {
      argv: ["cat", "/work/hello.txt"],
      env: {},
      stream: false,
    })
    if (isSandboxError(readResult)) {
      throw readResult
    }
    console.log(`${forkedId} -> ${new TextDecoder().decode(readResult.stdout).trim()}`)

    // Clean up forked session
    await client.destroySession(forkedId)
  }

  // Clean up original session
  await client.destroySession(sessionId)
  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
