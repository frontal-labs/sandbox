import { Client, isSandboxError, type SessionConfig } from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates running arbitrary code in a sandboxed Node.js environment.
 * Uses `node -e` to execute a JavaScript string directly.
 */
async function main(): Promise<void> {
  // Create client from environment variables (SANDBOX_BASE_URL, SANDBOX_API_KEY)
  const client = Client.fromEnv()

  // Session configuration
  const sessionConfig: SessionConfig = {
    tenant_id: "code-runner",
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

  // JavaScript code to execute - computes sum of array [1,2,3,4]
  const code = "console.log(JSON.stringify({ sum: [1,2,3,4].reduce((a,b)=>a+b,0) }))"

  // Execute the code using Node.js
  const execResult = await client.exec(sessionId, {
    argv: ["node", "-e", code],
    env: {},
    stream: false,
  })
  if (isSandboxError(execResult)) {
    throw execResult
  }

  // Output results
  console.log("Stdout:", new TextDecoder().decode(execResult.stdout).trim())
  console.log("Exit code:", execResult.exit_code)

  // Clean up the session
  await client.destroySession(sessionId)
  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
