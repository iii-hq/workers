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

export type DispatchResult = { kind: 'result'; result: FunctionResult } | { kind: 'pending' };

export function missingFunctionResult(): FunctionResult {
  return errorResult({
    error: 'missing_function',
    message: 'agent_trigger requires a non-empty `function` string field',
  });
}

/**
 * Some providers (and smaller models) emit a function-call argument that is a
 * JSON-*encoded string* rather than a JSON object — e.g. `payload` arrives as
 * `'{"sandbox_id":"…","cmd":"bash"}'` instead of `{ sandbox_id: …, cmd: … }`.
 * Forwarded verbatim, that string fails server-side deserialization with an
 * opaque `serialization error: missing field <first field>` (observed against
 * `sandbox::exec`, which then surfaced as a misleading `gate_unavailable`).
 *
 * Coerce: if `value` is a string whose trimmed form starts with `{`/`[` and
 * parses to an object/array, return the parsed value. A non-JSON string (or a
 * string that parses to a scalar) is left untouched so genuine string payloads
 * pass through unharmed.
 */
export function coercePayload(value: unknown): unknown {
  if (typeof value !== 'string') return value;
  const trimmed = value.trim();
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return value;
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed !== null && typeof parsed === 'object') return parsed;
  } catch {
    // Not valid JSON — leave the original string in place.
  }
  return value;
}

export function unwrapAgentTrigger(fc: FunctionCall): FunctionCall {
  const coerced = coercePayload(fc.arguments ?? {});
  const args = (coerced && typeof coerced === 'object' ? coerced : {}) as Record<string, unknown>;
  const fn = typeof args.function === 'string' ? args.function : '';
  const payload = coercePayload(args.payload ?? {});
  return { id: fc.id, function_id: fn, arguments: payload };
}

export function agentTriggerTool(): unknown {
  return {
    name: TOOL_NAME,
    description:
      'Call any iii function on the bus. The argument `function` is the function id (use `::` separators, e.g. `shell::fs::ls`). The argument `payload` is the function-specific arguments as a JSON object (an object literal — never a JSON-encoded string; do not stringify it). Skills loaded into your context tell you which functions exist and what arguments they take. The result is whatever that function returns.',
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
 * Extract the first complete, brace-balanced JSON object substring from `text`,
 * or `null` if there is none. Used to recover a structured error envelope that
 * the wire wraps behind a human prefix, e.g.
 * `invocation_failed: handler error: {"code":"S211","fix":{...},...}`.
 *
 * The scan is string-aware (braces inside JSON string values, and `\"` /
 * `\\` escapes, do not affect depth) so nested objects like `"fix":{...}` and
 * messages containing literal `{`/`}` are handled, and any trailing text after
 * the closing brace is ignored. Returns `null` on an unbalanced/truncated
 * object.
 */
export function extractFirstJsonObject(text: string): string | null {
  const start = text.indexOf('{');
  if (start === -1) return null;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i++) {
    const ch = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === '\\') escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === '{') depth++;
    else if (ch === '}') {
      depth--;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  return null;
}

/**
 * If `err` is an {@link IIIInvocationError} whose `.message` carries a
 * structured wire payload like
 * `{"code":"S210","type":"...","message":"...","docs_url":"...","fix":{...},"retryable":bool,...}`,
 * return that payload. Otherwise return `null`.
 *
 * Several workers (notably `iii-worker`'s `sandbox::*`) serialize their
 * domain errors as JSON via `Display` so the engine forwards them with the
 * structured envelope intact. Without this extractor, those payloads get
 * buried inside `gate_unavailable.reason` as `String(err)`, hiding the
 * S-code, the docs URL, the `fix` hint, and any structured retry info from
 * the calling agent.
 *
 * The wire message is not always pure JSON: the engine prepends a human
 * prefix such as `invocation_failed: handler error: ` before the `{`, so a
 * naive `JSON.parse` of the whole message fails and the fix hint stays buried.
 * We therefore try a direct parse first (fast path for pure-JSON messages),
 * then fall back to extracting the first brace-balanced JSON object embedded in
 * the message.
 *
 * Only payloads with both a `code` string and a `message` string are
 * accepted, so generic JSON values (numbers, plain strings, partial
 * envelopes) still fall through to the `gate_unavailable` path.
 */
function extractStructuredHandlerError(err: unknown): Record<string, unknown> | null {
  if (!(err instanceof IIIInvocationError)) return null;
  // `IIIInvocationError`'s Error.message may be `"${code}: ${rawMessage}"`. The
  // raw handler-emitted message lives after that prefix.
  const prefix = `${err.code}: `;
  const raw = err.message.startsWith(prefix) ? err.message.slice(prefix.length) : err.message;

  const candidates: string[] = [raw];
  // Fallback: the structured envelope is embedded after a human prefix
  // (e.g. `... handler error: {json}`). Pull the first balanced JSON object.
  const embedded = extractFirstJsonObject(raw);
  if (embedded && embedded !== raw) candidates.push(embedded);

  for (const candidate of candidates) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(candidate);
    } catch {
      continue;
    }
    if (!parsed || typeof parsed !== 'object') continue;
    const obj = parsed as Record<string, unknown>;
    if (typeof obj.code === 'string' && typeof obj.message === 'string') return obj;
  }
  return null;
}

/** Best-effort message string from an arbitrary thrown value. */
function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  if (err && typeof err === 'object') {
    const msg = (err as Record<string, unknown>).message;
    if (typeof msg === 'string') return msg;
  }
  return '';
}

/**
 * True when `err` is a payload-decoding failure on the worker/engine side: the
 * arguments could not be deserialized into the target function's request type.
 * These are CALLER errors (wrong shape, missing required field, double-encoded
 * JSON, base64 in the wrong field), not infrastructure failures, so they must
 * not be reported as `gate_unavailable` — that hid the real problem and sent
 * agents debugging the bus instead of their own arguments.
 */
export function isArgumentDecodeError(err: unknown): boolean {
  const msg = errorMessage(err);
  if (!msg) return false;
  return /serialization error|missing field|invalid type|unknown field|did not match any variant|expected .*(?:found|but)/i.test(
    msg,
  );
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

/**
 * Best-effort fetch of a function's contract (request/response schema,
 * description, owning worker) from the engine, used to enrich an
 * `invalid_arguments` error so the caller's NEXT attempt is correctly shaped
 * without having to call `engine::functions::info` itself.
 *
 * Deterministic and bounded: one trigger, no retries. Returns `null` on any
 * failure (so the caller falls back to the plain error) and never fetches the
 * contract OF the info call itself — that is pointless and risks a loop.
 * Prefers the structured `details` payload of the info response; falls back to
 * the raw value.
 */
export async function fetchContract(iii: ISdk, function_id: string): Promise<unknown | null> {
  if (!function_id || function_id === 'engine::functions::info') return null;
  try {
    const value = await iii.trigger<unknown, unknown>({
      function_id: 'engine::functions::info',
      payload: { function_id },
    });
    if (value && typeof value === 'object') {
      const details = (value as Record<string, unknown>).details;
      if (details !== undefined && details !== null) return details;
    }
    return value ?? null;
  } catch {
    return null;
  }
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
    // Payload-decode failures are caller errors, not infra outages: classify
    // them as `invalid_arguments` with an actionable hint instead of burying
    // them in `gate_unavailable`, which sent agents debugging the bus.
    if (isArgumentDecodeError(err)) {
      // Auto-attach the target's contract so the model's NEXT attempt is
      // correctly shaped without spending a turn on engine::functions::info.
      // Best-effort: `contract` is omitted if the fetch fails.
      const contract = await fetchContract(iii, function_call.function_id);
      const baseMessage =
        "`payload` could not be decoded into this function's arguments. Pass `payload` as a JSON OBJECT (an object literal — NOT a JSON-encoded string) whose fields match the schema: every required field, correct value types, and no field the schema does not define.";
      return errorResult({
        error: 'invalid_arguments',
        function: function_call.function_id,
        message: contract
          ? `${baseMessage} The function's contract is included below as \`contract\` — build your payload from it and retry once.`
          : `${baseMessage} Fetch the exact shape via engine::functions::info.`,
        detail: errorMessage(err) || String(err),
        ...(contract ? { contract } : {}),
      });
    }
    return denialResult(
      gateUnavailableEnvelope(function_call.function_id, `trigger_failed: ${String(err)}`),
    );
  }
}

/**
 * `function_call` carries the routing envelope (session id + call identity) that
 * the approval hook and policy worker read. `targetCall` is what actually
 * reaches the target function. They differ on purpose: the envelope spreads
 * routing fields (including `function_id` = the TARGET id) at the top level,
 * which would otherwise CLOBBER a same-named argument in the caller's payload —
 * e.g. `engine::functions::info { function_id: "sandbox::create" }` would arrive
 * as `function_id: "engine::functions::info"` and the engine would describe
 * itself. The hook needs the envelope; the target must get the caller's
 * untouched arguments. Defaults to `function_call` for callers that don't split.
 */
export async function dispatchWithHook(
  iii: ISdk,
  function_call: FunctionCall,
  targetCall: FunctionCall = function_call,
): Promise<DispatchResult> {
  const outcome = await consultBefore(iii, function_call);
  if (outcome.kind === 'deny') {
    return { kind: 'result', result: denialResult(outcome.denial) };
  }
  if (outcome.kind === 'pending') {
    return { kind: 'pending' };
  }

  const result = await triggerFunctionCall(iii, targetCall);
  return { kind: 'result', result };
}

const errorResultDetailsSchema = z.union([
  z.object({ error: z.string() }),
  z.object({ status: z.literal('denied') }),
]);

export function isErrorResult(result: FunctionResult): boolean {
  return errorResultDetailsSchema.safeParse(result.details).success;
}
