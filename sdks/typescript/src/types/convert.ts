/**
 * Shared helpers for converting between the server's snake_case wire format
 * and the camelCase surface the SDK exposes to callers.
 *
 * The server encodes every field in snake_case (see `crates/sandbox-types`).
 * Callers in TypeScript use camelCase, which is the idiomatic style for this
 * language.
 *
 * These helpers rename struct field names recursively, but leave the keys of
 * map-typed fields untouched: `env`, `labels`, and `secrets` hold arbitrary
 * user-supplied keys (environment variable names, label keys, secret names)
 * that the server treats as opaque strings, so they round-trip unchanged.
 */

/// `snake_case` → `camelCase` for a single known field name.
export function snakeToCamel(key: string): string {
  return key.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase())
}

/// `camelCase` → `snake_case` for a single known field name.
export function camelToSnake(key: string): string {
  return key.replace(/([A-Z])/g, (m, _p, offset: number) =>
    offset === 0 ? m : `_${m.toLowerCase()}`,
  )
}

function isPlainObject(value: unknown): boolean {
  if (value === null || typeof value !== "object") {
    return false
  }
  if (Array.isArray(value)) {
    return false
  }
  const proto = Object.getPrototypeOf(value)
  return proto === null || proto === Object.prototype
}

/// Field names whose values are maps with opaque user-supplied keys. The keys
/// of these maps (and the keys of plain objects nested inside them) must not
/// be renamed.
const OPAQUE_MAP_KEYS = new Set(["env", "labels", "secrets"])

/// Recursively rename struct keys. When entering a value whose parent key is
/// in `OPAQUE_MAP_KEYS`, the child object's keys are left alone (they are map
/// keys), though its values are still walked so nested structs convert.
function walk(value: unknown, rename: (k: string) => string, skipKeys: boolean): unknown {
  if (Array.isArray(value)) {
    return value.map((v) => walk(v, rename, false))
  }
  if (isPlainObject(value)) {
    const out: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      const newKey = skipKeys ? k : rename(k)
      out[newKey] = walk(v, rename, OPAQUE_MAP_KEYS.has(k))
    }
    return out
  }
  return value
}

/// Recursively rename struct keys from snake_case to camelCase. Map keys
/// (`env`, `labels`, `secrets`) are preserved.
export function toCamelCase(value: unknown): unknown {
  return walk(value, snakeToCamel, false)
}

/// Recursively rename struct keys from camelCase to snake_case. Map keys
/// (`env`, `labels`, `secrets`) are preserved.
export function toSnakeCase(value: unknown): unknown {
  return walk(value, camelToSnake, false)
}