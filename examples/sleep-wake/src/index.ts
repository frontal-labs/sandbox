import {
  Client,
  isSandboxError,
  type SessionConfig,
  type SessionState,
} from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates session sleep/wake for persistence:
 * create session -> write state -> sleep (persist to snapshot) -> wake (restore) -> verify state
 */
async function main(): Promise<void> {
  // Create client from environment variables (SANDBOX_BASE_URL, SANDBOX_API_KEY)
  const client = Client.fromEnv()

  // Session configuration
  const sessionConfig: SessionConfig = {
    tenantId: "sleep-wake",
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

  // Write persistent state to the filesystem
  const writeResult = await client.exec(sessionId, {
    argv: ["sh", "-c", "echo persisted > /work/state.txt"],
    env: {},
    stream: false,
  })
  if (isSandboxError(writeResult)) {
    throw writeResult
  }

  // Put session to sleep - creates a snapshot and stops the session
  // Returns the snapshot ID that the session can be woken from
  const sleepResult = await client.sleep(sessionId)
  if (isSandboxError(sleepResult)) {
    throw sleepResult
  }
  console.log("Slept, snapshot:", sleepResult)

  // Wake the session - restores filesystem from snapshot, shell state starts fresh
  const wakeResult = await client.wake(sessionId)
  if (isSandboxError(wakeResult)) {
    throw wakeResult
  }
  const _restoredState: SessionState = wakeResult

  // Verify the persisted state is restored
  const readResult = await client.exec(sessionId, {
    argv: ["cat", "/work/state.txt"],
    env: {},
    stream: false,
  })
  if (isSandboxError(readResult)) {
    throw readResult
  }
  console.log("After wake:", new TextDecoder().decode(readResult.stdout).trim())

  // Clean up the session
  await client.destroySession(sessionId)
  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
