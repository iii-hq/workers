/**
 * Zod schemas for the `workflow::*` namespace (durable multi-agent DAG
 * orchestrator).
 *
 * Wire source: `iii/workers/workflow/src/types.rs` + `src/functions/*.rs`
 *   - WorkflowDef / NodeDef / InputSpec (One | Many) / FanoutSpec / OutputRef
 *   - StartRequest / StartResponse              (start.rs)
 *   - StatusResponse (run record + node map)    (status.rs)
 *   - NodeResultRequest / NodeResultResponse    (node_result.rs)
 *
 * Schemas are deliberately lenient (passthrough, optional) so a payload with
 * extra/new fields still parses and renders rather than falling back to raw
 * JSON. Mirrors the worker:: / shell:: parser modules.
 */
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'

export { unwrapEnvelope }

export const WORKFLOW_FUNCTION_IDS = [
  'workflow::start',
  'workflow::status',
  'workflow::stop',
  'workflow::node-result',
  // Internal lifecycle ids — branded in the header label but rendered with the
  // default JSON pane (no bespoke view): they rarely surface in chat.
  'workflow::tick',
  'workflow::sweep',
  'workflow::wake',
] as const

export type WorkflowFunctionId = (typeof WORKFLOW_FUNCTION_IDS)[number]

const WORKFLOW_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  WORKFLOW_FUNCTION_IDS,
)

export function isWorkflowFunction(id: string): id is WorkflowFunctionId {
  return WORKFLOW_FUNCTION_ID_SET.has(id)
}

/* ---------------- WorkflowDef (the DAG) ---------------- */

/** `input.from` is a single source OR an array of `node:<id>` refs (the join
 *  form). Kept as the raw union; helpers normalise to an array for display. */
export const inputFromSchema = z.union([z.string(), z.array(z.string())])
export type InputFrom = z.infer<typeof inputFromSchema>

export const nodeDefSchema = z
  .object({
    agent: z
      .object({
        model: z.string(),
        provider: z.string().nullable().optional(),
        system_prompt: z.string().nullable().optional(),
      })
      .passthrough(),
    input: z
      .object({
        from: inputFromSchema,
        template: z.string().nullable().optional(),
      })
      .passthrough(),
    depends_on: z.array(z.string()).optional().default([]),
    fanout: z.object({ over: z.string() }).passthrough().nullable().optional(),
  })
  .passthrough()
export type WorkflowNodeDef = z.infer<typeof nodeDefSchema>

export const workflowDefSchema = z
  .object({
    version: z.number().optional(),
    nodes: z.record(z.string(), nodeDefSchema),
    output: z.object({ from: z.string() }).passthrough(),
  })
  .passthrough()
export type WorkflowDef = z.infer<typeof workflowDefSchema>

/* ---------------- workflow::start ---------------- */

export const startRequestSchema = z
  .object({
    definition: workflowDefSchema,
    input: z.unknown().optional(),
    idempotency_key: z.string().nullable().optional(),
    notify: z
      .object({
        function_id: z.string(),
        queue: z.string().nullable().optional(),
      })
      .passthrough()
      .nullable()
      .optional(),
  })
  .passthrough()
export type StartRequest = z.infer<typeof startRequestSchema>

export const startResponseSchema = z
  .object({ run_id: z.string() })
  .passthrough()
export type StartResponse = z.infer<typeof startResponseSchema>

/* ---------------- run status enum ---------------- */

export const runStatusSchema = z.enum([
  'awaiting_nodes',
  'running',
  'completed',
  'failed',
  'cancelled',
])
export type RunStatus = z.infer<typeof runStatusSchema>

/* ---------------- workflow::status ---------------- */

export const nodeStateSchema = z.enum([
  'pending',
  'running',
  'done',
  'failed',
  'cancelled',
])
export type NodeState = z.infer<typeof nodeStateSchema>

export const statusRequestSchema = z
  .object({ run_id: z.string() })
  .passthrough()
export type StatusRequest = z.infer<typeof statusRequestSchema>

export const statusResponseSchema = z
  .object({
    run_id: z.string().optional(),
    status: runStatusSchema.optional(),
    step: z.number().optional(),
    nodes: z.record(z.string(), nodeStateSchema).optional().default({}),
    node_results: z.record(z.string(), z.string()).optional().default({}),
    node_errors: z.record(z.string(), z.string()).nullable().optional(),
    result: z.unknown().optional(),
    result_error: z.string().nullable().optional(),
  })
  .passthrough()
export type StatusResponse = z.infer<typeof statusResponseSchema>

/* ---------------- workflow::node-result ---------------- */

export const nodeResultRequestSchema = z
  .object({ run_id: z.string(), node_uid: z.string() })
  .passthrough()
export type NodeResultRequest = z.infer<typeof nodeResultRequestSchema>

export const nodeResultResponseSchema = z
  .object({ result: z.unknown() })
  .passthrough()
export type NodeResultResponse = z.infer<typeof nodeResultResponseSchema>

/* ---------------- workflow::stop ---------------- */

export const stopRequestSchema = z.object({ run_id: z.string() }).passthrough()
export type StopRequest = z.infer<typeof stopRequestSchema>

export const stopResponseSchema = z
  .object({
    run_id: z.string().optional(),
    status: runStatusSchema.optional(),
    stopped: z.boolean().optional(),
  })
  .passthrough()
export type StopResponse = z.infer<typeof stopResponseSchema>

/* ---------------- DAG display helpers (pure) ---------------- */

/** Normalise `input.from` to an array of sources, dropping empties. */
export function inputSources(from: InputFrom | undefined): string[] {
  if (from == null) return []
  return (Array.isArray(from) ? from : [from]).filter((s) => s.length > 0)
}

/** The `node:<id>` dep ids a node consumes via input.from (strips prefix +
 *  dotted path). Non-`node:` sources (run_input/fanout_item) are excluded. */
export function consumedDeps(from: InputFrom | undefined): string[] {
  return inputSources(from)
    .filter((s) => s.startsWith('node:'))
    .map((s) => s.slice('node:'.length).split('.')[0])
}

/** A node is a JOIN when it reads more than one upstream (array form) or
 *  depends on more than one node. The console highlights these because they
 *  are where the multi-input wiring matters most. */
export function isJoinNode(node: WorkflowNodeDef): boolean {
  return (
    consumedDeps(node.input.from).length > 1 ||
    (node.depends_on?.length ?? 0) > 1
  )
}

export interface DagCounts {
  total: number
  done: number
  running: number
  pending: number
  failed: number
  cancelled: number
}

/** Tally node states from a status response's node map (uid → state). Fanned
 *  items (`#i`) each count individually, matching how the run actually executes. */
export function tallyNodes(nodes: Record<string, NodeState>): DagCounts {
  const counts: DagCounts = {
    total: 0,
    done: 0,
    running: 0,
    pending: 0,
    failed: 0,
    cancelled: 0,
  }
  for (const state of Object.values(nodes)) {
    counts.total += 1
    counts[state] += 1
  }
  return counts
}

/* ---------------- generic parse helpers (mirror worker/parsers) ---------------- */

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}

export function safeParseResponse<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(unwrapEnvelope(value))
  return parsed.success ? parsed.data : null
}
