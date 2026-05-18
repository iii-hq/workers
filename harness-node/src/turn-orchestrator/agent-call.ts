/**
 * `agent::call` dispatcher + chokepoint. Mirrors
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

export const TOOL_NAME = 'agent_call';
export const FUNCTION_ID = 'agent::call';

export type DispatchResult =
  | { kind: 'result'; result: FunctionResult }
  | { kind: 'deny'; result: FunctionResult }
  | { kind: 'pending' };

export function agentCallTool(): unknown {
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
    label: 'agent_call',
    execution_mode: 'parallel',
    prepare_arguments_supported: false,
  };
}

function errorResult(envelope: Record<string, unknown>): FunctionResult {
  const text: ContentBlock = { type: 'text', text: JSON.stringify(envelope) };
  return { content: [text], details: envelope, terminate: false };
}

function denialResult(envelope: DenialEnvelope | Record<string, unknown>): FunctionResult {
  const reasonRaw = (envelope as Record<string, unknown>).reason;
  const reason = typeof reasonRaw === 'string' ? reasonRaw : 'Permission denied.';
  return {
    content: [{ type: 'text', text: reason }],
    details: envelope,
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
  approval_required: string[],
  session_id: string | undefined,
  policy_function_id: string,
): Promise<DispatchResult> {
  if (!function_call.function_id || function_call.function_id.length === 0) {
    return {
      kind: 'result',
      result: errorResult({
        error: 'missing_function',
        message: 'agent_call requires a non-empty `function` string field',
      }),
    };
  }
  const outcome = await consultBefore(
    iii,
    function_call,
    approval_required,
    session_id,
    policy_function_id,
  );
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
          hint: 'load the relevant skill via directory::skills::get, or check the function id',
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
  policy_function_id: string,
): Promise<FunctionResult> {
  if (typeof fn !== 'string' || fn.length === 0) {
    return errorResult({
      error: 'missing_function',
      message: 'agent_call requires a non-empty `function` string field',
    });
  }
  const fc: FunctionCall = {
    id: `agent_call-${uuidLike()}`,
    function_id: fn,
    arguments: payload ?? {},
  };
  const out = await dispatchWithHook(iii, fc, [], session_id, policy_function_id);
  if (out.kind === 'pending') {
    return errorResult({
      error: 'awaiting_approval',
      function: fc.function_id,
      message: 'This call requires human approval. Approve via the console and retry.',
    });
  }
  return out.result;
}

export function register(iii: ISdk, policy_function_id: string): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const session_id = typeof obj.session_id === 'string' ? obj.session_id : '';
      const fn = obj.function;
      const inner = obj.payload ?? {};
      return await dispatch(iii, session_id, fn, inner, policy_function_id);
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
