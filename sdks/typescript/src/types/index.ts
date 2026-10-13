import { z } from "zod"

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
  secrets: z.record(SecretEntrySchema),
})
export type SecretsConfig = z.infer<typeof SecretsConfigSchema>

export const SessionConfigSchema = z.object({
  tenantId: z.string().min(1),
  tierPreference: TierPreferenceSchema.default("auto"),
  limits: ResourceLimitsSchema.default({}),
  network: NetworkPolicySchema.default({ defaultDeny: true, rules: [] }),
  env: z.record(z.string()).default({}),
  cwd: z.string().default("/work"),
  initialSnapshot: z.string().optional(),
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
  tenantId: z.string().min(1),
  tier: TierSchema,
  status: SessionStatusSchema,
  configHash: z.string().min(1),
  vfsRoot: z.string().optional(),
  createdAt: z.string().datetime(),
  updatedAt: z.string().datetime(),
  labels: z.record(z.string()),
})
export type SessionState = z.infer<typeof SessionStateSchema>

export const ExecRequestSchema = z.object({
  argv: z.array(z.string().min(1)).min(1),
  env: z.record(z.string()).default({}),
  cwd: z.string().optional(),
  stdin: z.instanceof(Uint8Array).optional(),
  timeoutMs: z.number().int().positive().optional(),
  stream: z.boolean().default(false),
})
export type ExecRequest = z.infer<typeof ExecRequestSchema>

export const ExecResultSchema = z.object({
  exitCode: z.number().int(),
  stdout: z.instanceof(Uint8Array),
  stderr: z.instanceof(Uint8Array),
  cpuMs: z.number().int().nonnegative(),
  wallMs: z.number().int().nonnegative(),
})
export type ExecResult = z.infer<typeof ExecResultSchema>

export const SnapshotSchema = z.object({
  id: z.string().min(1),
  parent: z.string().optional(),
  vfsRoot: z.string().min(1),
  env: z.record(z.string()),
  cwd: z.string().min(1),
  tier: TierSchema,
  runtimeBlob: z.string().optional(),
  createdAt: z.string().datetime(),
  labels: z.record(z.string()),
})
export type Snapshot = z.infer<typeof SnapshotSchema>

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
} as const