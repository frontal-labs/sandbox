import { z } from "zod"

// Field names are the server's wire names (snake_case). The server encodes
// timestamps as RFC 3339 strings and byte fields as arrays of 0-255 integers.

export const TierSchema = z.enum(["isolate", "vm"])
export type Tier = z.infer<typeof TierSchema>

export const TierPreferenceSchema = z.enum(["auto", "isolate", "vm"])
export type TierPreference = z.infer<typeof TierPreferenceSchema>

export const ResourceLimitsSchema = z.object({
  memory_bytes: z.number().int().positive().optional(),
  cpu_ms_per_sec: z.number().int().positive().optional(),
  wall_timeout_secs: z.number().int().positive().optional(),
  max_file_size: z.number().int().positive().optional(),
  max_total_size: z.number().int().positive().optional(),
  max_inodes: z.number().int().positive().optional(),
  max_path_depth: z.number().int().positive().optional(),
  max_processes: z.number().int().positive().optional(),
  max_open_files: z.number().int().positive().optional(),
})
export type ResourceLimits = z.infer<typeof ResourceLimitsSchema>

export const NetworkPolicyRuleSchema = z.object({
  action: z.enum(["allow", "deny"]),
  host_pattern: z.string().min(1),
  ports: z.array(z.number().int().positive().max(65535)).optional(),
})
export type NetworkPolicyRule = z.infer<typeof NetworkPolicyRuleSchema>

export const NetworkPolicySchema = z.object({
  default_deny: z.boolean(),
  rules: z.array(NetworkPolicyRuleSchema),
})
export type NetworkPolicy = z.infer<typeof NetworkPolicySchema>

export const SecretEntrySchema = z.object({
  value: z.string(),
  hosts: z.array(z.string()),
})
export type SecretEntry = z.infer<typeof SecretEntrySchema>

export const SecretsConfigSchema = z.object({
  secrets: z.record(SecretEntrySchema),
})
export type SecretsConfig = z.infer<typeof SecretsConfigSchema>

export const SessionConfigSchema = z.object({
  tenant_id: z.string().min(1),
  tier_preference: TierPreferenceSchema.default("auto"),
  limits: ResourceLimitsSchema.default({}),
  network: NetworkPolicySchema.default({ default_deny: true, rules: [] }),
  env: z.record(z.string()).default({}),
  cwd: z.string().default("/work"),
  initial_snapshot: z.string().optional(),
  secrets: SecretsConfigSchema.optional(),
  labels: z.record(z.string()).default({}),
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
  tenant_id: z.string().min(1),
  tier: TierSchema,
  status: SessionStatusSchema,
  config_hash: z.string().min(1),
  vfs_root: z.string().optional(),
  created_at: z.string().datetime(),
  updated_at: z.string().datetime(),
  labels: z.record(z.string()),
})
export type SessionState = z.infer<typeof SessionStateSchema>

/// A byte field as the server sends it: an array of integers 0-255. Parsed
/// into a `Uint8Array` for callers.
const ByteArraySchema = z
  .array(z.number().int().min(0).max(255))
  .transform((bytes) => Uint8Array.from(bytes))

export const ExecRequestSchema = z.object({
  argv: z.array(z.string().min(1)).min(1),
  env: z.record(z.string()).default({}),
  cwd: z.string().optional(),
  stdin: z.instanceof(Uint8Array).optional(),
  timeout_ms: z.number().int().positive().optional(),
  stream: z.boolean().default(false),
})
export type ExecRequest = z.infer<typeof ExecRequestSchema>

export const ExecResultSchema = z.object({
  exit_code: z.number().int(),
  stdout: ByteArraySchema,
  stderr: ByteArraySchema,
  cpu_ms: z.number().int().nonnegative(),
  wall_ms: z.number().int().nonnegative(),
})
export type ExecResult = z.infer<typeof ExecResultSchema>

export const SnapshotSchema = z.object({
  id: z.string().min(1),
  parent: z.string().optional(),
  vfs_root: z.string().min(1),
  env: z.record(z.string()),
  cwd: z.string().min(1),
  tier: TierSchema,
  runtime_blob: z.string().optional(),
  created_at: z.string().datetime(),
  labels: z.record(z.string()),
})
export type Snapshot = z.infer<typeof SnapshotSchema>

/// The body of `POST /v1/sessions`, which returns the new ID.
export const CreateSessionResponseSchema = z.object({
  session_id: z.string().min(1),
})

/// The body of `POST /v1/snapshots/{id}/fork`.
export const ForkResponseSchema = z.object({
  session_ids: z.array(z.string().min(1)),
})

/// The body of `POST /v1/sessions/{id}/snapshot`.
export const TakeSnapshotResponseSchema = z.object({
  snapshot_id: z.string().min(1),
})

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
