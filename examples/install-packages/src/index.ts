import { Client, isSandboxError, type SessionConfig } from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates installing system packages in a sandbox session.
 * Note: This example simulates package installation since the sandbox
 * may not have a package manager pre-installed depending on the tier.
 */
async function main(): Promise<void> {
  // Create client from environment variables (SANDBOX_BASE_URL, SANDBOX_API_KEY)
  const client = Client.fromEnv()

  // Session configuration
  const sessionConfig: SessionConfig = {
    tenantId: "install-packages",
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

  // Execute a command to simulate package installation
  // In a real scenario, you might use: apt-get update && apt-get install -y <package>
  const execResult = await client.exec(sessionId, {
    argv: ["sh", "-c", "echo 'simulated package install' && which sh"],
    env: {},
    stream: false,
  })
  if (isSandboxError(execResult)) {
    throw execResult
  }

  console.log("Output:", new TextDecoder().decode(execResult.stdout))

  // Clean up the session
  await client.destroySession(sessionId)
  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
