// llama-server Chat Completions streaming. Kept separate from
// provider-openai / provider-kimi / provider-lmstudio so llama.cpp-
// specific extensions (cache_prompt, slot_id, --jinja knobs) can be
// added without coupling the providers.
//
// Compared to provider-lmstudio/stream.ts this is a leaner generator:
//   - No `lmstudio-local` placeholder resolution (llama-server runs one
//     model; the model id is whatever the user typed and the server
//     accepts any string for the single loaded model).
//   - No auto-load retry block. llama-server can't load a different
//     model at runtime — it would require restarting the process with
//     a different `-m`.
//   - All other defenses (timeout, non-2xx truncation, EOF-without-
//     finish_reason guard, error event short-circuit) are preserved.

import { logger } from '../runtime/otel.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';
import type { AgentFunction } from '../types/function.js';
import type { AssistantMessageEvent } from '../types/stream-event.js';
import {
  buildFinal,
  classifyLlamacppError,
  emptyPartial,
  handleChunk,
  syntheticErrorEvent,
} from './sse.js';
import type { ChatCompletionsConfig } from './types.js';
import { toOpenaiMessages } from './wire-messages.js';
import { functionsToOpenai } from './wire-tools.js';

/**
 * Catalog placeholder id. Maps to "whatever llama-server has loaded";
 * the request forwards this string verbatim — llama-server accepts any
 * model name for its single loaded model.
 */
export const PLACEHOLDER_MODEL_ID = 'llamacpp-local';

/**
 * Default connect + first-byte timeout for any llama-server HTTP call.
 *
 * Generous because large GGUFs (35B+ Q4) can spend tens of seconds on
 * prompt processing before the first token arrives — especially when
 * llama-server is on a remote LAN host and the prompt is long. A
 * too-short timeout surfaces as "RESPONSE FAILED: LLAMACPP FETCH
 * TIMED OUT" mid-think.
 *
 * Override per-machine via `LLAMACPP_FETCH_TIMEOUT_MS` (any positive
 * integer in milliseconds; values <1000 are treated as a
 * misconfiguration and ignored).
 */
export const DEFAULT_FETCH_TIMEOUT_MS = 120_000;

export function resolveFetchTimeoutMs(): number {
  const raw = (process.env.LLAMACPP_FETCH_TIMEOUT_MS ?? '').trim();
  if (raw.length === 0) return DEFAULT_FETCH_TIMEOUT_MS;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || parsed < 1_000) return DEFAULT_FETCH_TIMEOUT_MS;
  return Math.floor(parsed);
}

export type StreamArgs = {
  cfg: ChatCompletionsConfig;
  system_prompt: string;
  messages: AgentMessage[];
  tools: AgentFunction[];
};

/**
 * Run `fetch` with an AbortController-based timeout. Without this a
 * misconfigured URL hangs for ~75s (macOS SYN timeout) and the UI
 * stays stuck on "thinking…" the whole time.
 */
async function fetchWithTimeout(
  url: string,
  init: RequestInit,
  timeoutMs: number,
): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

export async function* streamLlamacpp({
  cfg,
  system_prompt,
  messages,
  tools,
}: StreamArgs): AsyncGenerator<AssistantMessageEvent> {
  const authName = cfg.auth_header_name ?? 'Authorization';
  const authPrefix = cfg.auth_value_prefix ?? 'Bearer ';
  const headers: Record<string, string> = {
    'content-type': 'application/json',
  };
  if (cfg.api_key.length > 0) {
    headers[authName] = `${authPrefix}${cfg.api_key}`;
  }
  for (const [k, v] of cfg.extra_headers ?? []) headers[k] = v;

  const effectiveModel = cfg.model;
  const body: Record<string, unknown> = {
    model: effectiveModel,
    max_completion_tokens: cfg.max_tokens,
    messages: toOpenaiMessages(messages, system_prompt),
    stream: true,
    stream_options: { include_usage: true },
  };
  if (tools.length > 0) body.tools = functionsToOpenai(tools);

  // Resolve the timeout fresh on each call so tests can mutate the
  // env var between cases, and so an operator who sets
  // `LLAMACPP_FETCH_TIMEOUT_MS` at runtime (e.g. via a config
  // reload) sees the new value without restarting the worker.
  const fetchTimeoutMs = resolveFetchTimeoutMs();
  let resp: Response;
  try {
    resp = await fetchWithTimeout(
      cfg.url,
      { method: 'POST', headers, body: JSON.stringify(body) },
      fetchTimeoutMs,
    );
  } catch (err) {
    const msg =
      err instanceof Error && err.name === 'AbortError'
        ? `llamacpp fetch timed out after ${fetchTimeoutMs / 1000}s (is ${cfg.url} reachable?)`
        : `llamacpp fetch failed: ${String(err)}`;
    yield syntheticErrorEvent(msg, effectiveModel, cfg.provider_name);
    return;
  }
  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    // Truncate + strip control chars before surfacing — non-2xx
    // responses can carry HTML, proxy error pages, or attacker-
    // controlled bodies. Length cap keeps the message readable;
    // control-char strip avoids ANSI / newline injection.
    const safeText = text
      // eslint-disable-next-line no-control-regex
      .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')
      .slice(0, 256);
    yield syntheticErrorEvent(
      safeText || `llamacpp http ${resp.status}`,
      cfg.model,
      cfg.provider_name,
      classifyLlamacppError(safeText, resp.status),
    );
    return;
  }
  const partial: AssistantMessage = {
    role: 'assistant',
    content: [],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model: effectiveModel,
    provider: cfg.provider_name,
    timestamp: Date.now(),
  };
  yield { type: 'start', partial };

  const state = emptyPartial();
  if (!resp.body) {
    yield syntheticErrorEvent(
      'llamacpp response missing body',
      effectiveModel,
      cfg.provider_name,
    );
    return;
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  let parsedChunkCount = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      let idx = buf.indexOf('\n\n');
      while (idx >= 0) {
        const block = buf.slice(0, idx);
        buf = buf.slice(idx + 2);
        const dataLine = parseDataLine(block);
        idx = buf.indexOf('\n\n');
        if (dataLine === null) continue;
        if (dataLine === '[DONE]') {
          yield {
            type: 'done',
            message: buildFinal(state, effectiveModel, cfg.provider_name),
          };
          return;
        }
        let parsed: Record<string, unknown> | null = null;
        try {
          parsed = JSON.parse(dataLine) as Record<string, unknown>;
        } catch {
          continue;
        }
        if (parsed) {
          parsedChunkCount++;
          for (const e of handleChunk(parsed, state, effectiveModel, cfg.provider_name)) {
            yield e;
            // handleChunk returns an `error` event when llama-server
            // sent an SSE error chunk. Stop reading immediately so
            // the rest of the body (typically just a connection
            // close) doesn't fall into the EOF-without-finish_reason
            // guard and overwrite the server's specific error.
            if (e.type === 'error') return;
          }
        }
      }
    }
  } catch (err) {
    logger.warn('llamacpp stream read failed', { err: String(err) });
    yield syntheticErrorEvent(
      `stream read failed: ${String(err)}`,
      effectiveModel,
      cfg.provider_name,
    );
    return;
  }
  if (parsedChunkCount === 0) {
    yield syntheticErrorEvent(
      'llamacpp returned a 200 response with a non-SSE body (no parseable chunks). The endpoint URL is likely wrong, or the server served an HTML page in place of an SSE stream.',
      cfg.model,
      cfg.provider_name,
    );
    return;
  }
  // We parsed some chunks but never saw `data: [DONE]` AND never saw a
  // `finish_reason`. The server closed the body mid-response — typical
  // causes: GPU OOM, context exhausted, host disconnect. Promote to an
  // explicit error so the UI doesn't show a silently truncated reply.
  if (!state.saw_finish_reason) {
    const partial = buildFinal(state, effectiveModel, cfg.provider_name);
    const tokenHint =
      partial.usage && (partial.usage.output ?? 0) > 0
        ? ` after ~${partial.usage.output} output tokens`
        : '';
    yield syntheticErrorEvent(
      `llamacpp stream closed mid-response${tokenHint} — the server ended the SSE body without a [DONE] marker or finish_reason. Common causes: GPU OOM, host disconnect, or context exhausted during generation.`,
      effectiveModel,
      cfg.provider_name,
      'transient',
    );
    return;
  }
  yield { type: 'done', message: buildFinal(state, effectiveModel, cfg.provider_name) };
}

function parseDataLine(block: string): string | null {
  let data: string | null = null;
  for (const line of block.split('\n')) {
    if (line.startsWith('data: ')) data = line.slice('data: '.length);
  }
  return data;
}

export async function collect(
  events: AsyncIterable<AssistantMessageEvent>,
): Promise<AssistantMessage> {
  let last: AssistantMessage | null = null;
  for await (const ev of events) {
    if (ev.type === 'done') return ev.message;
    if (ev.type === 'error') return ev.error;
    if ('partial' in ev) last = ev.partial;
  }
  return (
    last ?? {
      role: 'assistant',
      content: [],
      stop_reason: 'error',
      error_message: 'stream closed without final',
      error_kind: 'transient',
      usage: null,
      model: 'llamacpp',
      provider: 'llamacpp',
      timestamp: Date.now(),
    }
  );
}
