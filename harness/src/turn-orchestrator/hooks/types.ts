/**
 * Wire types for the `harness::hook::pre-trigger` trigger type: the
 * hook input/output contract (harness.md § Hooks › Contract) and the
 * per-binding config schema. Hook owners (e.g. the approval-gate
 * worker's `approval::gate`) bind to the trigger type and answer
 * continue / deny / hold synchronously.
 */

import { z } from 'zod';

export const PRE_TRIGGER_TYPE = 'harness::hook::pre-trigger';

/** Fail-closed default budget for one hook consult. */
export const DEFAULT_HOOK_TIMEOUT_MS = 5_000;

export type HookCall = {
  id: string;
  function_id: string;
  arguments: unknown;
};

export type HookInput = {
  point: 'pre_trigger';
  session_id: string;
  turn_id: string;
  step?: number;
  /** Sub-agent depth (0 = top-level). The harness has no nesting yet. */
  depth: number;
  metadata?: Record<string, unknown>;
  call: HookCall;
};

/**
 * Hook reply. Anything that fails this parse is a transport-level
 * failure and follows the binding's `on_error` policy (fail closed by
 * default) — a confused hook must never read as an allow.
 */
export const HookOutputSchema = z.discriminatedUnion('decision', [
  z.object({ decision: z.literal('continue') }),
  z.object({ decision: z.literal('deny'), reason: z.string() }),
  z.object({ decision: z.literal('hold'), pending_timeout_ms: z.number().int().nonnegative() }),
]);
export type HookOutput = z.infer<typeof HookOutputSchema>;

/**
 * Per-binding config. `.strict()` so a misspelled filter key fails at
 * registration, not silently (mirrors the gate's `deny_unknown_fields`).
 */
export const BindingConfigSchema = z
  .object({
    /** Globs narrowing which triggers consult this hook; absent/empty = all. */
    functions: z.array(z.string().min(1)).optional(),
    /** Chain position; lower runs first. Default 0, ties by function_id. */
    priority: z.number().int().optional(),
    timeout_ms: z.number().int().positive().default(DEFAULT_HOOK_TIMEOUT_MS),
    on_error: z.enum(['fail_closed', 'fail_open']).default('fail_closed'),
  })
  .strict();
export type BindingConfig = z.infer<typeof BindingConfigSchema>;

/** Turn a glob pattern (`*` = any substring) into an anchored regex. */
function compileFunctionGlob(pattern: string): RegExp {
  let re = '^';
  for (const ch of pattern) {
    if (ch === '*') re += '.*';
    else re += ch.replace(/[\\^$.|?+()[\]{}]/g, '\\$&');
  }
  re += '$';
  return new RegExp(re);
}

export function globMatch(pattern: string, function_id: string): boolean {
  if (!pattern.includes('*')) return pattern === function_id;
  return compileFunctionGlob(pattern).test(function_id);
}
