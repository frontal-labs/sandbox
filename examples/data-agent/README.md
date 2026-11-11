# Frontal Sandbox Data Agent Example

Demonstrates a data agent workflow using the Frontal Sandbox TypeScript SDK: load a dataset into the sandbox, run an analysis, and write results out.

## Prerequisites

- Node.js 20+
- A running Frontal Sandbox server (default `http://localhost:8080`)

## Run

```bash
npm install
npm start
```

## Environment

- `SANDBOX_BASE_URL` (default `http://localhost:8080`)
- `SANDBOX_API_KEY` (optional)

## What it demonstrates

1. Creates a sandbox session.
2. Loads a bundled CSV dataset into the session filesystem via `writeFile`.
3. Runs an analysis in Node.js (`node -e`) that computes summary statistics and writes `/work/results.json`.
4. Reads the results back via `readFile` to confirm they were written out.
5. Destroys the session.

## Python WASM

The analysis step currently runs in Node.js because CPython as a WASI tool is not yet installed into sessions (see `docs/PYTHON_WASM.md`, open item 1). Once the Python module and stdlib are available in a session, the analysis can switch to `python /work/analyze.py` or `python -c`.
