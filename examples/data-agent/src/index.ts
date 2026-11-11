import { Client, isSandboxError, type SessionConfig } from "@frontal-labs/sandbox-sdk"

/**
 * Demonstrates a data agent workflow: load a dataset into the sandbox, run an
 * analysis, and write results out. Uses Node.js (available now). When Python
 * WASM is installed into sessions (docs/PYTHON_WASM.md open item 1), the
 * analysis step can switch to `python /work/analyze.py`.
 */
async function main(): Promise<void> {
  const client = Client.fromEnv()

  const sessionConfig: SessionConfig = {
    tenantId: "data-agent",
    tierPreference: "auto",
    cwd: "/work",
    limits: {},
    network: { defaultDeny: true, rules: [] },
    env: {},
    labels: {},
  }

  const sessionResult = await client.createSession(sessionConfig)
  if (isSandboxError(sessionResult)) {
    throw sessionResult
  }
  const sessionId = sessionResult

  try {
    // Load a bundled dataset into the session filesystem.
    const csv = [
      "region,product,amount,quantity",
      "north,widget,120,3",
      "south,gadget,200,5",
      "north,gadget,150,2",
      "east,widget,90,4",
      "south,widget,110,1",
    ].join("\n")
    const writeCsv = await client.writeFile(sessionId, "/work/data.csv", new TextEncoder().encode(csv))
    if (isSandboxError(writeCsv)) {
      throw writeCsv
    }

    // Run analysis in Node.js: compute summary stats and write results.json.
    const analysis = `
const fs = require('fs');
const lines = fs.readFileSync('/work/data.csv', 'utf8').trim().split('\\n');
const rows = lines.slice(1).map(line => {
  const [region, product, amount, quantity] = line.split(',');
  return { region, product, amount: Number(amount), quantity: Number(quantity) };
});
const amounts = rows.map(r => r.amount);
const totalAmount = amounts.reduce((a, b) => a + b, 0);
const result = {
  rowCount: rows.length,
  totalAmount,
  meanAmount: totalAmount / rows.length,
  totalQuantity: rows.reduce((a, r) => a + r.quantity, 0),
  byRegion: rows.reduce((acc, r) => {
    acc[r.region] = (acc[r.region] || 0) + r.amount;
    return acc;
  }, {})
};
fs.writeFileSync('/work/results.json', JSON.stringify(result, null, 2));
console.log(JSON.stringify({ status: 'ok', rowCount: result.rowCount }));
`.trim()

    const execResult = await client.exec(sessionId, {
      argv: ["node", "-e", analysis],
      env: {},
      stream: false,
    })
    if (isSandboxError(execResult)) {
      throw execResult
    }
    if (execResult.exitCode !== 0) {
      throw new Error(
        `analysis failed (exit ${execResult.exitCode}): ${new TextDecoder().decode(execResult.stderr)}`,
      )
    }

    console.log("Analysis:", new TextDecoder().decode(execResult.stdout).trim())

    // Read results back from the session to confirm they were written out.
    const resultsFile = await client.readFile(sessionId, "/work/results.json")
    if (isSandboxError(resultsFile)) {
      throw resultsFile
    }
    const results = JSON.parse(new TextDecoder().decode(resultsFile)) as {
      rowCount: number
      totalAmount: number
      meanAmount: number
    }
    console.log("Results:", JSON.stringify(results))
    console.log(
      `Summary: ${results.rowCount} rows, total=${results.totalAmount}, mean=${results.meanAmount}`,
    )

    if (results.rowCount !== 5 || results.totalAmount !== 670) {
      throw new Error(`unexpected analysis results: ${JSON.stringify(results)}`)
    }
  } finally {
    const destroyed = await client.destroySession(sessionId)
    if (isSandboxError(destroyed)) {
      throw destroyed
    }
  }

  console.log("Done")
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
