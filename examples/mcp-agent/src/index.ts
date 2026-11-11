import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process"
import { once } from "node:events"
import { existsSync } from "node:fs"
import { resolve } from "node:path"
import { createInterface } from "node:readline"

type JsonRpcId = number
type JsonRpcMessage = {
  jsonrpc?: string
  id?: JsonRpcId
  method?: string
  params?: unknown
  result?: unknown
  error?: { code: number; message: string }
}

class McpClient {
  private readonly child: ChildProcessWithoutNullStreams
  private readonly pending = new Map<
    JsonRpcId,
    { resolve: (v: JsonRpcMessage) => void; reject: (e: Error) => void }
  >()
  private nextId = 1
  private closed = false

  constructor(bin: string) {
    this.child = spawn(bin, ["mcp"], { stdio: ["pipe", "pipe", "pipe"] })
    const rl = createInterface({ input: this.child.stdout })
    rl.on("line", (line) => this.onLine(line))
    this.child.on("exit", (code, signal) => {
      this.closed = true
      const err = new Error(`mcp process exited (code=${code} signal=${signal})`)
      for (const [, p] of this.pending) p.reject(err)
      this.pending.clear()
    })
    this.child.stderr.on("data", (d) => {
      const text = d.toString().trim()
      if (text) console.error("[mcp stderr]", text)
    })
  }

  private onLine(line: string): void {
    const trimmed = line.trim()
    if (!trimmed) return
    let msg: JsonRpcMessage
    try {
      msg = JSON.parse(trimmed) as JsonRpcMessage
    } catch {
      console.error("[mcp non-json]", trimmed)
      return
    }
    if (msg.id === undefined || msg.id === null) return
    const pending = this.pending.get(msg.id)
    if (!pending) return
    this.pending.delete(msg.id)
    pending.resolve(msg)
  }

  private request(method: string, params: unknown): Promise<JsonRpcMessage> {
    if (this.closed) return Promise.reject(new Error("mcp process is closed"))
    const id = this.nextId++
    const msg = { jsonrpc: "2.0", id, method, params }
    const p = new Promise<JsonRpcMessage>((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
    })
    this.child.stdin.write(JSON.stringify(msg) + "\n")
    return p
  }

  private notify(method: string): void {
    if (this.closed) return
    this.child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method }) + "\n")
  }

  async initialize(): Promise<void> {
    const reply = await this.request("initialize", {})
    if (reply.error) throw new Error(`initialize error: ${reply.error.message}`)
    const result = reply.result as { protocolVersion?: string } | undefined
    if (result?.protocolVersion !== "2024-11-05") {
      throw new Error(`unexpected protocolVersion: ${String(result?.protocolVersion)}`)
    }
    this.notify("notifications/initialized")
  }

  async callTool(name: string, args: Record<string, string>): Promise<string> {
    const reply = await this.request("tools/call", { name, arguments: args })
    if (reply.error) throw new Error(`protocol error: ${reply.error.message}`)
    const result = reply.result as { isError?: boolean; content?: { text?: string }[] }
    const text = (result.content ?? []).map((c) => c.text ?? "").join("\n")
    if (result.isError) throw new Error(`tool ${name} failed: ${text}`)
    return text
  }

  async close(): Promise<void> {
    if (this.closed) return
    this.child.stdin.end()
    await Promise.race([once(this.child, "exit"), new Promise((r) => setTimeout(r, 2000))])
    if (!this.closed) this.child.kill()
  }
}

function assertCondition(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(msg)
}

function resolveBinary(): string {
  const bin = resolve(process.env.FRONTAL_SANDBOX_BIN ?? "../../target/debug/frontal-sandbox")
  if (!existsSync(bin)) {
    throw new Error(`frontal-sandbox binary not found: ${bin}`)
  }
  return bin
}

async function main(): Promise<void> {
  const bin = resolveBinary()
  const client = new McpClient(bin)
  try {
    await client.initialize()
    console.log("[agent] initialized MCP session")

    const listing = await client.callTool("list_dir", { path: "/" })
    console.log("[agent] list_dir / ->", listing.replace(/\n/g, ", ") || "(empty)")

    const buggySource = [
      "add() {",
      "  echo $(( $1 - $2 ))",
      "}",
      "",
    ].join("\n")
    await client.callTool("write_file", { path: "/math.sh", content: buggySource })
    console.log("[agent] wrote /math.sh (buggy)")

    const tests = [
      "fail=0",
      '[ "$(add 2 3)" = "5" ] || { echo "FAIL: add 2 3"; fail=1; }',
      '[ "$(add -1 1)" = "0" ] || { echo "FAIL: add -1 1"; fail=1; }',
      'if [ "$fail" -ne 0 ]; then exit 1; fi',
      "echo ok",
      "",
    ].join("\n")
    await client.callTool("write_file", { path: "/math.test.sh", content: tests })
    console.log("[agent] wrote /math.test.sh")

    const runTests = 'sh -c "$(cat /math.sh; cat /math.test.sh)"'
    const before = await client.callTool("exec", { command: runTests })
    console.log("[agent] test run (before fix):")
    console.log(before)
    assertCondition(before.includes("[exit 1]") || before.includes("FAIL"), "expected tests to fail before fix")

    const readBack = await client.callTool("read_file", { path: "/math.sh" })
    assertCondition(readBack.includes("$1 - $2"), "expected buggy implementation to be present")

    const fixedSource = [
      "add() {",
      "  echo $(( $1 + $2 ))",
      "}",
      "",
    ].join("\n")
    await client.callTool("write_file", { path: "/math.sh", content: fixedSource })
    console.log("[agent] fixed /math.sh")

    const after = await client.callTool("exec", { command: runTests })
    console.log("[agent] test run (after fix):")
    console.log(after)
    assertCondition(after.includes("ok"), "expected tests to print ok after fix")
    assertCondition(after.includes("[exit 0]"), "expected exit code 0 after fix")

    console.log("MCP coding agent smoke test passed")
  } finally {
    await client.close()
  }
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : String(err))
  process.exit(1)
})
