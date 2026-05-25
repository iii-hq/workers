/**
 * Agent function-call dispatcher + approval chokepoint.
 *
 * `dispatchWithHook` is the single chokepoint for FSM-issued calls: every
 * agent function call goes through `consultBefore` before reaching the inner
 * trigger. `triggerFunctionCall` is the shared trigger/decode/error path
 * used by both the hook gate and pre-approved resume execution.
 */

import { IIIInvocationError, type ISdk } from '../runtime/iii.js';
import { z } from 'zod';
import type { ContentBlock } from '../types/content.js';
import type { FunctionCall, FunctionResult } from '../types/function.js';
import { type DenialEnvelope, consultBefore, gateUnavailableEnvelope } from './hook.js';

export const TOOL_NAME = 'agent_trigger';

export type DispatchResult =
  | { kind: 'result'; result: FunctionResult }
  | { kind: 'deny'; result: FunctionResult }
  | { kind: 'pending' };

export function missingFunctionResult(): FunctionResult {
  return errorResult({
    error: 'missing_function',
    message: 'agent_trigger requires a non-empty `function` string field',
  });
}

export function unwrapAgentTrigger(fc: FunctionCall): FunctionCall {
  if (fc.function_id !== TOOL_NAME) return fc;
  const args = (fc.arguments ?? {}) as Record<string, unknown>;
  const fn = typeof args.function === 'string' ? args.function : '';
  const payload = args.payload ?? {};
  return { id: fc.id, function_id: fn, arguments: payload };
}

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
 * If `err` is an {@link IIIInvocationError} whose `.message` carries a
 * structured wire payload like
 * `{"code":"S210","type":"...","message":"...","docs_url":"...","retryable":bool,...}`,
 * return that payload. Otherwise return `null`.
 *
 * Several workers (notably `iii-worker`'s `sandbox::*`) serialize their
 * domain errors as JSON via `Display` so the engine forwards them with the
 * structured envelope intact. Without this extractor, those payloads get
 * buried inside `gate_unavailable.reason` as `String(err)`, hiding the
 * S-code, the docs URL, the `fix` hint, and any structured retry info from
 * the calling agent.
 *
 * Only payloads with both a `code` string and a `message` string are
 * accepted, so generic JSON values (numbers, plain strings, partial
 * envelopes) still fall through to the `gate_unavailable` path.
 */
function extractStructuredHandlerError(err: unknown): Record<string, unknown> | null {
  if (!(err instanceof IIIInvocationError)) return null;
  // `IIIInvocationError`'s Error.message is `"${code}: ${rawMessage}"`. The
  // raw handler-emitted message lives after that prefix.
  const prefix = `${err.code}: `;
  const raw = err.message.startsWith(prefix) ? err.message.slice(prefix.length) : err.message;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== 'object') return null;
  const obj = parsed as Record<string, unknown>;
  if (typeof obj.code !== 'string' || typeof obj.message !== 'string') return null;
  return obj;
}

export function functionNotFoundHint(badFunctionId: string): string {
  if (!badFunctionId.includes('/')) {
    return 'load the relevant skill via directory::skills::get, or check the function id';
  }
  const generic =
    'Skill ids are NOT function ids. `agent_trigger` expects the function id ' +
    '(`worker::fn`) — that is the `function_id` field on each row returned by ' +
    "`directory::skills::list`, not the row's `id` field (which is the on-disk " +
    'skill path).';
  const segments = badFunctionId.split('/').filter((s) => s.length > 0);
  let suggestion: string | null = null;
  if (segments.length >= 4 && segments[1] === 'skills' && segments[0] === segments[2]) {
    suggestion = `${segments[0]}::${segments.slice(3).join('::')}`;
  } else if (segments.length === 2 && segments[1] !== 'index') {
    suggestion = `${segments[0]}::${segments[1]}`;
  }
  return suggestion ? `Did you mean \`${suggestion}\`? ${generic}` : generic;
}

/** Trigger a function call and normalize success/error into a FunctionResult. */
export async function triggerFunctionCall(
  iii: ISdk,
  function_call: FunctionCall,
): Promise<FunctionResult> {
  try {
    const value = await iii.trigger<unknown, unknown>({
      function_id: function_call.function_id,
      payload: function_call.arguments ?? {},
    });
    return decodeOrPassthrough(value);
  } catch (err) {
    if (isFunctionNotFound(err)) {
      return errorResult({
        error: 'function_not_found',
        function: function_call.function_id,
        hint: functionNotFoundHint(function_call.function_id),
      });
    }
    // Structured worker error (e.g. `sandbox::*` S-codes): forward the
    // envelope verbatim so the agent gets code + docs_url + fix hint
    // instead of a generic gate_unavailable wrapper. We tag it with
    // `error: 'handler_error'` so `isErrorResult` (and any downstream
    // "did this tool call fail?" gate) still classifies it correctly.
    const structured = extractStructuredHandlerError(err);
    if (structured) {
      return errorResult({ error: 'handler_error', ...structured });
    }
    return denialResult(
      gateUnavailableEnvelope(function_call.function_id, `trigger_failed: ${String(err)}`),
    );
  }
}

export async function dispatchWithHook(
  iii: ISdk,
  function_call: FunctionCall,
): Promise<DispatchResult> {
  const outcome = await consultBefore(iii, function_call);
  if (outcome.kind === 'deny') {
    return { kind: 'deny', result: denialResult(outcome.denial) };
  }
  if (outcome.kind === 'pending') {
    return { kind: 'pending' };
  }

  const result = await triggerFunctionCall(iii, function_call);
  return { kind: 'result', result };
}

const errorResultDetailsSchema = z.union([
  z.object({ error: z.string() }),
  z.object({ status: z.literal('denied') }),
]);

export function isErrorResult(result: FunctionResult): boolean {
  return errorResultDetailsSchema.safeParse(result.details).success;
}
