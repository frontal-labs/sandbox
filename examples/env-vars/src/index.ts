import { Client, isSandboxError, type SessionConfig } from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates passing environment variables to a sandbox session.
 * Environment variables set in the session config are available to all exec commands.
 */
async function main(): Promise<void> {
  // Create client from environment variables (SANDBOX_BASE_URL, SANDBOX_API_KEY)
  const client = Client.fromEnv()

  // Session configuration with custom environment variables
  const sessionConfig: SessionConfig = {
    tenantId: "env-vars",
    tierPreference: "auto",
    cwd: "/work",
    limits: {},
    network: { defaultDeny: true, rules: [] },
    env: { GREETING: "hello", COUNT: "42" },
    labels: {},
  }

  // Create a new sandbox session with the configured environment
  const sessionResult = await client.createSession(sessionConfig)
  if (isSandboxError(sessionResult)) {
    throw sessionResult
  }
  const sessionId = sessionResult

  // Execute a command that reads the environment variables
  const execResult = await client.exec(sessionId, {
    argv: ["sh", "-c", "echo $GREETING $COUNT"],
    env: {}, // Additional env vars for this specific exec (merged with session env)
    stream: false,
  })
  if (isSandboxError(execResult)) {
    throw execResult
  }

  // Output the result - should print "hello 42"
  console.log(new TextDecoder().decode(execResult.stdout).trim())

  // Clean up the session
  await client.destroySession(sessionId)
  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
