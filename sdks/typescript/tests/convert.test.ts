import { test } from "vitest"
import { wireSchemas, wireResponseSchemas } from "../src/types/index.js"

test("env var names and label keys are not mangled", () => {
  const wire = wireSchemas.SessionConfig.parse({
    tenantId: "t",
    tierPreference: "isolate",
    limits: {},
    network: { defaultDeny: true, rules: [] },
    env: { PATH: "/usr/bin", MY_VAR: "hello", FOO_BAR: "x" },
    cwd: "/work",
    labels: { "my-label": "v", UPPER_KEY: "2" },
  })
  // User-supplied map keys (env var names, label keys) must round-trip unchanged.
  expect(wire.env).toEqual({ PATH: "/usr/bin", MY_VAR: "hello", FOO_BAR: "x" })
  expect(wire.labels).toEqual({ "my-label": "v", UPPER_KEY: "2" })
})

test("response map keys are not mangled", () => {
  const parsed = wireResponseSchemas.SessionState.parse({
    id: "x",
    tenant_id: "t",
    tier: "isolate",
    status: "running",
    config_hash: "0".repeat(64),
    created_at: "2026-10-15T12:00:00.000Z",
    updated_at: "2026-10-15T12:00:00.000Z",
    labels: { UPPER_KEY: "2", "my-label": "v" },
  })
  expect(parsed.labels).toEqual({ UPPER_KEY: "2", "my-label": "v" })
})