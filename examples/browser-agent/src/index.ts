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
  const bin = resolve(
    process.env.FRONTAL_SANDBOX_BIN ?? "../../target/debug/frontal-sandbox",
  )
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

    const page = [
      "<!doctype html>",
      "<html>",
      "<head><title>Sandbox Browser Demo</title></head>",
      "<body>",
      "  <h1>Welcome</h1>",
      "  <p id=\"status\">ready</p>",
      "  <ul>",
      "    <li>alpha</li>",
      "    <li>beta</li>",
      "  </ul>",
      "</body>",
      "</html>",
      "",
    ].join("\n")
    await client.callTool("write_file", { path: "/page.html", content: page })
    console.log("[agent] wrote /page.html")

    const run = await client.callTool("exec", {
      command: [
        "title=$(sed -n 's/.*<title>\\([^<]*\\)<\\/title>.*/\\1/p' /page.html | head -n 1)",
        "status=$(sed -n 's/.*id=\"status\">\\([^<]*\\)<.*/\\1/p' /page.html | head -n 1)",
        "item1=$(sed -n 's/.*<li>\\([^<]*\\)<\\/li>.*/\\1/p' /page.html | head -n 1)",
        "item2=$(sed -n 's/.*<li>\\([^<]*\\)<\\/li>.*/\\1/p' /page.html | tail -n 1)",
        "printf '{\"title\":\"%s\",\"status\":\"%s\",\"items\":[\"%s\",\"%s\"]}\\n' \"$title\" \"$status\" \"$item1\" \"$item2\"",
      ].join("; "),
    })
    console.log("[agent] browser extraction result:")
    console.log(run)

    assertCondition(run.includes("[exit 0]"), "expected extractor to exit 0")
    const jsonLine = run.split("\n").find((l) => l.trim().startsWith("{"))
    assertCondition(jsonLine, "expected JSON output from extractor")
    const data = JSON.parse(jsonLine) as { title: string; status: string; items: string[] }
    assertCondition(data.title === "Sandbox Browser Demo", "expected page title")
    assertCondition(data.status === "ready", "expected status ready")
    assertCondition(
      data.items.length === 2 && data.items[0] === "alpha" && data.items[1] === "beta",
      "expected list items alpha,beta",
    )

    console.log("Browser agent smoke test passed")
  } finally {
    await client.close()
  }
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : String(err))
  process.exit(1)
})
