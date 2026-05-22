/**
 * `agent::trigger` dispatcher + chokepoint. Mirrors
 * `turn-orchestrator/src/agent_call.rs`.
 *
 * `dispatchWithHook` is the single chokepoint: every agent-issued tool
 * call goes through `consultBefore` before reaching the inner trigger.
 * Fail-closed: a hook timeout / error / missing subscriber denies the
 * call with a `gate_unavailable` envelope (Phase 2.B §F).
 */

import { uuidLike } from '../runtime/ids.js';
import type { ISdk } from '../runtime/iii.js';
import type { ContentBlock } from '../types/content.js';
import type { FunctionCall, FunctionResult } from '../types/function.js';
import { type DenialEnvelope, consultBefore, gateUnavailableEnvelope } from './hook.js';

export const TOOL_NAME = 'agent_trigger';
export const FUNCTION_ID = 'agent::trigger';

export type DispatchResult =
  | { kind: 'result'; result: FunctionResult }
  | { kind: 'deny'; result: FunctionResult }
  | { kind: 'pending' };

export function agentTriggerTool(): unknown {
  return {
    name: TOOL_NAME,
    description:
      'Call any iii function on the bus. The argument `function` is the function id (use `::` separators, e.g. `shell::fs::ls`). The argument `payload` is the function-specific JSON arguments. Skills loaded into your context tell you which functions exist and what arguments they take. The result is whatever that function returns.',
    parameters: {
      type: 'object',
      properties: {
        function: {
          type: 'string',
          description: "iii function id to dispatch, e.g. 'shell::fs::ls'.",
        },
        payload: {
          type: 'object',
          description: 'Arguments forwarded to the function. Shape depends on the target function.',
        },
      },
      required: ['function'],
    },
    label: 'agent_trigger',
    execution_mode: 'parallel',
    prepare_arguments_supported: false,
  };
}

function errorResult(envelope: Record<string, unknown>): FunctionResult {
  const text: ContentBlock = { type: 'text', text: JSON.stringify(envelope) };
  return { content: [text], details: envelope, terminate: false };
}

function denialResult(denial: DenialEnvelope): FunctionResult {
  return {
    content: [{ type: 'text', text: denial.reason }],
    details: denial,
    terminate: false,
  };
}

function decodeOrPassthrough(value: unknown): FunctionResult {
  if (
    value &&
    typeof value === 'object' &&
    Array.isArray((value as Record<string, unknown>).content)
  ) {
    const obj = value as Record<string, unknown>;
    return {
      content: obj.content as ContentBlock[],
      details: obj.details ?? {},
      terminate: typeof obj.terminate === 'boolean' ? obj.terminate : false,
    };
  }
  let text: string;
  if (typeof value === 'string') text = value;
  else text = JSON.stringify(value);
  return {
    content: [{ type: 'text', text }],
    details: value,
    terminate: false,
  };
}

function isFunctionNotFound(err: unknown): boolean {
  if (!err || typeof err !== 'object') return false;
  const obj = err as Record<string, unknown>;
  if (obj.code === 'function_not_found') return true;
  if (typeof obj.message === 'string' && /function_not_found/i.test(obj.message)) return true;
  return false;
}

/**
 * Build the `hint` field on a `function_not_found` result. Models
 * regularly confuse the SKILL id (`sandbox/skills/sandbox/create`, the
 * on-disk path returned by `directory::skills::list`) with the FUNCTION
 * id (`sandbox::create`, what `agent_trigger` actually expects) and
 * then retry the same wrong id 3+ times before recovering. When the
 * caller's `function_id` contains a `/` we can usually reconstruct the
 * canonical worker::fn form and surface it as a "did you mean" — that
 * collapses the typical 4-turn recovery to a 2-turn recovery.
 *
 * Cases recognised:
 * - `<w>/skills/<w>/<fn...>`  → `<w>::<fn...>`  (the canonical skill-id
 *   shape produced by `directory::skills::list` for a how-to that
 *   declares `function_id:` in its frontmatter).
 * - `<w>/<fn>`                → `<w>::<fn>`     (weaker guess, but
 *   matches what models often hallucinate as a shorthand).
 *
 * Anything else (no `/`, or shapes we can't confidently rewrite) gets
 * the generic hint pointing at the skills surface.
 */
export function functionNotFoundHint(badFunctionId: string): string {
  if (!badFunctionId.includes('/')) {
    return 'load the relevant skill via directory::skills::get, or check the function id';
  }
  const generic =
    'Skill ids are NOT function ids. `agent_trigger` expects the function id ' +
    '(`worker::fn`) — that is the `function_id` field on each row returned by ' +
    '`directory::skills::list`, not the row\'s `id` field (which is the on-disk ' +
    'skill path).';
  const segments = badFunctionId.split('/').filter((s) => s.length > 0);
  let suggestion: string | null = null;
  if (
    segments.length >= 4 &&
    segments[1] === 'skills' &&
    segments[0] === segments[2]
  ) {
    // sandbox/skills/sandbox/create → sandbox::create
    // worker-a/skills/worker-a/nested/fn → worker-a::nested::fn
    suggestion = `${segments[0]}::${segments.slice(3).join('::')}`;
  } else if (segments.length === 2 && segments[1] !== 'index') {
    // sandbox/create → sandbox::create  (also catches accidental
    // `<worker>/<fn>` shorthand the model invented from the skill path)
    suggestion = `${segments[0]}::${segments[1]}`;
  }
  return suggestion ? `Did you mean \`${suggestion}\`? ${generic}` : generic;
}

function isTimeout(err: unknown): boolean {
  if (!err || typeof err !== 'object') return false;
  const obj = err as Record<string, unknown>;
  if (obj.code === 'timeout') return true;
  if (typeof obj.message === 'string' && /^Timeout|timed out/.test(obj.message)) return true;
  return false;
}

export async function dispatchWithHook(
  iii: ISdk,
  function_call: FunctionCall,
  session_id: string | undefined,
): Promise<DispatchResult> {
  if (!function_call.function_id || function_call.function_id.length === 0) {
    return {
      kind: 'result',
      result: errorResult({
        error: 'missing_function',
        message: 'agent_trigger requires a non-empty `function` string field',
      }),
    };
  }
  const outcome = await consultBefore(iii, function_call);
  if (outcome.kind === 'deny') return { kind: 'deny', result: denialResult(outcome.denial) };
  if (outcome.kind === 'pending') {
    return { kind: 'pending' };
  }

  try {
    const value = await iii.trigger<unknown, unknown>({
      function_id: function_call.function_id,
      payload: function_call.arguments ?? {},
    });
    return { kind: 'result', result: decodeOrPassthrough(value) };
  } catch (err) {
    if (isFunctionNotFound(err)) {
      return {
        kind: 'result',
        result: errorResult({
          error: 'function_not_found',
          function: function_call.function_id,
          hint: functionNotFoundHint(function_call.function_id),
        }),
      };
    }
    if (isTimeout(err)) {
      return {
        kind: 'result',
        result: errorResult({
          error: 'timeout',
          function: function_call.function_id,
          message: String(err),
        }),
      };
    }
    return {
      kind: 'deny',
      result: denialResult(
        gateUnavailableEnvelope(function_call.function_id, `trigger_failed: ${String(err)}`),
      ),
    };
  }
}

export async function dispatch(
  iii: ISdk,
  session_id: string,
  fn: unknown,
  payload: unknown,
): Promise<FunctionResult> {
  if (typeof fn !== 'string' || fn.length === 0) {
    return errorResult({
      error: 'missing_function',
      message: 'agent_trigger requires a non-empty `function` string field',
    });
  }
  const fc: FunctionCall = {
    id: `agent_trigger-${uuidLike()}`,
    function_id: fn,
    arguments: payload ?? {},
  };
  const out = await dispatchWithHook(iii, fc, session_id);
  if (out.kind === 'pending') {
    return errorResult({
      error: 'awaiting_approval',
      function: fc.function_id,
      message: 'This call requires human approval. Approve via the console and retry.',
    });
  }
  return out.result;
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = typeof obj.session_id === 'string' ? obj.session_id : '';
      const fn = obj.function;
      const inner = obj.payload ?? {};
      return await dispatch(iii, session_id, fn, inner);
    },
    {
      description:
        'LLM-facing dispatcher: dispatches an iii function and returns a FunctionResult.',
    },
  );
}

export function isErrorResult(result: FunctionResult): boolean {
  const details = result.details;
  if (!details || typeof details !== 'object') return false;
  const obj = details as Record<string, unknown>;
  if (typeof obj.error === 'string') return true;
  if (obj.status === 'denied') return true;
  return false;
}
