import { z } from "zod"
import { snakeToCamel, camelToSnake, toCamelCase, toSnakeCase } from "./convert.js"

export const TierSchema = z.enum(["isolate", "vm"])
export type Tier = z.infer<typeof TierSchema>

export const TierPreferenceSchema = z.enum(["auto", "isolate", "vm"])
export type TierPreference = z.infer<typeof TierPreferenceSchema>

export const ResourceLimitsSchema = z.object({
  memoryBytes: z.number().int().positive().optional(),
  cpuMsPerSec: z.number().int().positive().optional(),
  wallTimeoutSecs: z.number().int().positive().optional(),
  maxFileSize: z.number().int().positive().optional(),
  maxTotalSize: z.number().int().positive().optional(),
  maxInodes: z.number().int().positive().optional(),
  maxPathDepth: z.number().int().positive().optional(),
  maxProcesses: z.number().int().positive().optional(),
  maxOpenFiles: z.number().int().positive().optional(),
})
export type ResourceLimits = z.infer<typeof ResourceLimitsSchema>

export const NetworkPolicyRuleSchema = z.object({
  action: z.enum(["allow", "deny"]),
  hostPattern: z.string().min(1),
  ports: z.array(z.number().int().positive().max(65535)).optional(),
})
export type NetworkPolicyRule = z.infer<typeof NetworkPolicyRuleSchema>

export const NetworkPolicySchema = z.object({
  defaultDeny: z.boolean(),
  rules: z.array(NetworkPolicyRuleSchema),
})
export type NetworkPolicy = z.infer<typeof NetworkPolicySchema>

export const SecretEntrySchema = z.object({
  value: z.string(),
  hosts: z.array(z.string()),
})
export type SecretEntry = z.infer<typeof SecretEntrySchema>

export const SecretsConfigSchema = z.object({
  secrets: z.record(z.string(), SecretEntrySchema),
})
export type SecretsConfig = z.infer<typeof SecretsConfigSchema>

export const SessionConfigSchema = z.object({
  tenantId: z.string().min(1),
  tierPreference: TierPreferenceSchema.default("auto"),
  limits: ResourceLimitsSchema.default({}),
  network: NetworkPolicySchema.default({ defaultDeny: true, rules: [] }),
  env: z.record(z.string(), z.string()).default({}),
  cwd: z.string().default("/work"),
  initialSnapshot: z.string().optional(),
  secrets: SecretsConfigSchema.optional(),
  labels: z.record(z.string(), z.string()).default({}),
})
export type SessionConfig = z.infer<typeof SessionConfigSchema>

export const SessionStatusSchema = z.enum([
  "creating",
  "running",
  "sleeping",
  "promoting",
  "destroyed",
  "error",
])
export type SessionStatus = z.infer<typeof SessionStatusSchema>

export const SessionStateSchema = z.object({
  id: z.string().min(1),
  tenantId: z.string().min(1),
  tier: TierSchema,
  status: SessionStatusSchema,
  configHash: z.string().min(1),
  vfsRoot: z.string().nullable().optional(),
  createdAt: z.string().datetime(),
  updatedAt: z.string().datetime(),
  labels: z.record(z.string(), z.string()),
})
export type SessionState = z.infer<typeof SessionStateSchema>

const ByteArraySchema = z
  .array(z.number().int().min(0).max(255))
  .transform((bytes) => Uint8Array.from(bytes))

export const ExecRequestSchema = z.object({
  argv: z.array(z.string().min(1)).min(1),
  env: z.record(z.string(), z.string()).default({}),
  cwd: z.string().optional(),
  stdin: z.instanceof(Uint8Array).optional(),
  timeoutMs: z.number().int().positive().optional(),
  stream: z.boolean().default(false),
})
export type ExecRequest = z.infer<typeof ExecRequestSchema>

export const ExecResultSchema = z.object({
  exitCode: z.number().int(),
  stdout: ByteArraySchema,
  stderr: ByteArraySchema,
  cpuMs: z.number().int().nonnegative(),
  wallMs: z.number().int().nonnegative(),
})
export type ExecResult = z.infer<typeof ExecResultSchema>

export const SnapshotSchema = z.object({
  id: z.string().min(1),
  parent: z.string().optional(),
  vfsRoot: z.string().min(1),
  env: z.record(z.string(), z.string()),
  cwd: z.string().min(1),
  tier: TierSchema,
  runtimeBlob: z.string().optional(),
  createdAt: z.string().datetime(),
  labels: z.record(z.string(), z.string()),
})
export type Snapshot = z.infer<typeof SnapshotSchema>

export const CreateSessionResponseSchema = z.object({
  sessionId: z.string().min(1),
})

export const ForkResponseSchema = z.object({
  sessionIds: z.array(z.string().min(1)),
})

export const TakeSnapshotResponseSchema = z.object({
  snapshotId: z.string().min(1),
})

export const ForkRequestSchema = z.object({
  count: z.number().int().positive(),
})
export type ForkRequest = z.infer<typeof ForkRequestSchema>

// The server encodes timestamps as RFC 3339 strings and byte fields as arrays
// of 0-255 integers. Field names below are the camelCase surface the SDK
// exposes; the `wire` variants map them to the server's snake_case for HTTP
// transport (see `crates/sandbox-types`).

export const ExecRequestWireSchema = ExecRequestSchema.transform((req) => {
  const wire = toSnakeCase(req) as Record<string, unknown>
  if (wire["stdin"] !== undefined) {
    wire["stdin"] = Array.from(wire["stdin"] as Uint8Array)
  }
  return wire
})
export type ExecRequestWire = z.infer<typeof ExecRequestWireSchema>

export const wireSchemas = {
  SessionConfig: SessionConfigSchema.transform((data) => toSnakeCase(data)),
  ExecRequest: ExecRequestWireSchema,
  ForkRequest: ForkRequestSchema.transform((data) => toSnakeCase(data)),
}

export function parseResponse<T>(schema: z.ZodType<T>, data: unknown): T {
  return schema.parse(toCamelCase(data))
}

export const wireResponseSchemas = {
  SessionState: z.preprocess((data) => toCamelCase(data), SessionStateSchema),
  ExecResult: z.preprocess((data) => toCamelCase(data), ExecResultSchema),
  Snapshot: z.preprocess((data) => toCamelCase(data), SnapshotSchema),
  CreateSessionResponse: z.preprocess((data) => toCamelCase(data), CreateSessionResponseSchema),
  ForkResponse: z.preprocess((data) => toCamelCase(data), ForkResponseSchema),
  TakeSnapshotResponse: z.preprocess((data) => toCamelCase(data), TakeSnapshotResponseSchema),
}

export function generateId(): string {
  return crypto.randomUUID().replace(/-/g, "").substring(0, 26).toUpperCase()
}

export function nilId(): string {
  return "00000000000000000000000000"
}

export const schemas = {
  Tier: TierSchema,
  TierPreference: TierPreferenceSchema,
  ResourceLimits: ResourceLimitsSchema,
  NetworkPolicyRule: NetworkPolicyRuleSchema,
  NetworkPolicy: NetworkPolicySchema,
  SecretEntry: SecretEntrySchema,
  SecretsConfig: SecretsConfigSchema,
  SessionConfig: SessionConfigSchema,
  SessionStatus: SessionStatusSchema,
  SessionState: SessionStateSchema,
  ExecRequest: ExecRequestSchema,
  ExecResult: ExecResultSchema,
  Snapshot: SnapshotSchema,
}