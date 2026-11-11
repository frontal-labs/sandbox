import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process"
import { once } from "node:events"
import { existsSync } from "node:fs"
import { resolve } from "node:path"
import { createInterface } from "node:readline"

type RpcMsg = {
  jsonrpc?: string
  id?: number
  method?: string
  params?: unknown
  result?: unknown
  error?: { code: number; message: string }
}

type Pending = { resolve: (v: RpcMsg) => void; reject: (e: Error) => void }

function startMcp(binPath: string) {
  const child = spawn(binPath, ["mcp"], { stdio: ["pipe", "pipe", "pipe"] })
  const pending = new Map<number, Pending>()
  let seq = 1
  let closed = false

  const failAll = (err: Error) => {
    closed = true
    for (const [, p] of pending) p.reject(err)
    pending.clear()
  }

  createInterface({ input: child.stdout }).on("line", (line) => {
    const trimmed = line.trim()
    if (!trimmed) return
    let msg: RpcMsg
    try {
      msg = JSON.parse(trimmed) as RpcMsg
    } catch {
      console.error("[mcp non-json]", trimmed)
      return
    }
    if (msg.id == null) return
    const p = pending.get(msg.id)
    if (!p) return
    pending.delete(msg.id)
    p.resolve(msg)
  })

  child.on("exit", (code, signal) => {
    failAll(new Error(`mcp process exited (code=${code} signal=${signal})`))
  })
  child.stderr.on("data", (d) => {
    const text = d.toString().trim()
    if (text) console.error("[mcp stderr]", text)
  })

  const request = (method: string, params: unknown): Promise<RpcMsg> => {
    if (closed) return Promise.reject(new Error("mcp process is closed"))
    const id = seq++
    const payload = { jsonrpc: "2.0", id, method, params }
    const p = new Promise<RpcMsg>((resolve, reject) => pending.set(id, { resolve, reject }))
    child.stdin.write(`${JSON.stringify(payload)}\n`)
    return p
  }

  const notify = (method: string): void => {
    if (closed) return
    child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method })}\n`)
  }

  const initialize = async (): Promise<void> => {
    const reply = await request("initialize", {})
    if (reply.error) throw new Error(`initialize error: ${reply.error.message}`)
    const result = reply.result as { protocolVersion?: string } | undefined
    if (result?.protocolVersion !== "2024-11-05") {
      throw new Error(`unexpected protocolVersion: ${String(result?.protocolVersion)}`)
    }
    notify("notifications/initialized")
  }

  const callTool = async (name: string, args: Record<string, string>): Promise<string> => {
    const reply = await request("tools/call", { name, arguments: args })
    if (reply.error) throw new Error(`protocol error: ${reply.error.message}`)
    const result = reply.result as { isError?: boolean; content?: { text?: string }[] }
    const text = (result.content ?? []).map((c) => c.text ?? "").join("\n")
    if (result.isError) throw new Error(`tool ${name} failed: ${text}`)
    return text
  }

  const close = async (): Promise<void> => {
    if (closed) return
    child.stdin.end()
    await Promise.race([once(child, "exit"), new Promise((r) => setTimeout(r, 2000))])
    if (!closed) child.kill()
  }

  return { initialize, callTool, close }
}

function assertCondition(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(msg)
}

function resolveBinary(): string {
  const bin = resolve(process.env.FRONTAL_SANDBOX_BIN ?? "../../target/debug/frontal-sandbox")
  if (!existsSync(bin)) throw new Error(`frontal-sandbox binary not found: ${bin}`)
  return bin
}

const bin = resolveBinary()
const mcp = startMcp(bin)

try {
  await mcp.initialize()
  console.log("[agent] initialized MCP session")

  const page = [
    "<!doctype html>",
    "<html>",
    "<head><title>Sandbox Browser Demo</title></head>",
    "<body>",
    '  <h1>Welcome</h1>',
    '  <p id="status">ready</p>',
    "  <ul>",
    "    <li>alpha</li>",
    "    <li>beta</li>",
    "  </ul>",
    "</body>",
    "</html>",
    "",
  ].join("\n")
  await mcp.callTool("write_file", { path: "/page.html", content: page })
  console.log("[agent] wrote /page.html")

  const run = await mcp.callTool("exec", {
    command: [
      String.raw`title=$(sed -n 's/.*<title>\([^<]*\)<\/title>.*/\1/p' /page.html | head -n 1)`,
      String.raw`status=$(sed -n 's/.*id="status">\([^<]*\)<.*/\1/p' /page.html | head -n 1)`,
      String.raw`item1=$(sed -n 's/.*<li>\([^<]*\)<\/li>.*/\1/p' /page.html | head -n 1)`,
      String.raw`item2=$(sed -n 's/.*<li>\([^<]*\)<\/li>.*/\1/p' /page.html | tail -n 1)`,
      String.raw`printf '{"title":"%s","status":"%s","items":["%s","%s"]}\n' "$title" "$status" "$item1" "$item2"`,
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
  await mcp.close()
}

