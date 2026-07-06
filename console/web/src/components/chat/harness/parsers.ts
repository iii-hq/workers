/**
 * Zod schemas + helpers for the harness tool family.
 *
 * Wire sources:
 *   workers/harness/src/functions/spawn.rs  -> SpawnRequest / SpawnOptions
 *                                              SpawnResponse (direct call)
 *   workers/harness/src/deferred.rs         -> resolve_parent (pending result:
 *                                              { content, details } envelope)
 *   workers/harness/tests/golden/schemas/harness.spawn.json
 *
 * Schemas are non-strict so additive wire fields don't break the UI, and
 * optionals are `.nullish()` — serde skips `None` but model-emitted JSON may
 * carry explicit nulls. `task` is required on the wire but optional here: a
 * gated call's preview input can be a clipped `arguments_excerpt`.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

/* Synthetic + namespaced harness tools. `submit_result` is the
   output-contract fallback the harness injects into a turn;
   `harness::spawn` is the sub-agent pending trigger. */
export const HARNESS_FUNCTION_IDS = ['submit_result', 'harness::spawn'] as const
export type HarnessFunctionId = (typeof HARNESS_FUNCTION_IDS)[number]

const HARNESS_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  HARNESS_FUNCTION_IDS,
)

export function isHarnessFunction(id: string): id is HarnessFunctionId {
  return HARNESS_FUNCTION_ID_SET.has(id)
}

/* ---------------- request ---------------- */

export const spawnModeSchema = z.enum(['plan', 'ask', 'agent'])
export type SpawnMode = z.infer<typeof spawnModeSchema>

export const thinkingLevelSchema = z.enum([
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
])

export const systemPromptStrategySchema = z.enum(['override', 'enrich'])

export const outputContractSchema = z.union([
  z.object({ type: z.literal('text') }),
  z.object({ type: z.literal('json'), schema: z.unknown().optional() }),
])
export type OutputContract = z.infer<typeof outputContractSchema>

export const functionPolicySchema = z.object({
  allow: z.array(z.string()).optional(),
  deny: z.array(z.string()).optional(),
  expose: z.enum(['agent_trigger', 'native']).optional(),
})
export type FunctionPolicy = z.infer<typeof functionPolicySchema>

export const spawnOptionsSchema = z.object({
  system_prompt: z.string().nullish(),
  system_prompt_strategy: systemPromptStrategySchema.nullish(),
  mode: spawnModeSchema.nullish(),
  max_turns: z.number().nullish(),
  thinking_level: thinkingLevelSchema.nullish(),
  output: outputContractSchema.nullish(),
  functions: functionPolicySchema.nullish(),
  max_children: z.number().nullish(),
  pending_timeout_ms: z.number().nullish(),
})
export type SpawnOptions = z.infer<typeof spawnOptionsSchema>

/** `task` is string sugar or a full AgentMessage — we only read content[].text. */
export const taskMessageSchema = z
  .object({
    role: z.string().optional(),
    content: z.array(z.unknown()).optional(),
  })
  .passthrough()

export const spawnTaskSchema = z.union([z.string(), taskMessageSchema])
export type SpawnTask = z.infer<typeof spawnTaskSchema>

export const spawnRequestSchema = z.object({
  task: spawnTaskSchema.optional(),
  model: z.string().nullish(),
  provider: z.string().nullish(),
  session_id: z.string().nullish(),
  parent_session_id: z.string().nullish(),
  /* harness-stamped on react-fired spawns, not caller-supplied */
  spawned_by_subscription_id: z.string().nullish(),
  reactive_depth: z.number().nullish(),
  options: spawnOptionsSchema.nullish(),
})
export type SpawnRequest = z.infer<typeof spawnRequestSchema>

/* ---------------- response ---------------- */

/** Direct-call acknowledgement. The common agent_trigger path instead
    resolves to the child's raw result value (free-form) — no schema. */
export const spawnResponseSchema = z.object({
  child_session_id: z.string(),
  child_turn_id: z.string(),
})
export type SpawnResponse = z.infer<typeof spawnResponseSchema>

/* ---------------- helpers ---------------- */

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}

/** Flatten a task to display text: string passes through; an AgentMessage
    joins its `content[].text` blocks. */
export function taskText(task: SpawnTask | undefined | null): string | null {
  if (task == null) return null
  if (typeof task === 'string') return task.length > 0 ? task : null
  const parts: string[] = []
  for (const block of task.content ?? []) {
    if (!block || typeof block !== 'object') continue
    const obj = block as Record<string, unknown>
    if (
      obj.type === 'text' &&
      typeof obj.text === 'string' &&
      obj.text.length > 0
    ) {
      parts.push(obj.text)
    }
  }
  return parts.length > 0 ? parts.join('\n') : null
}
