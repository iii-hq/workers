// Kept separate from provider-openai / provider-kimi so LM Studio-specific
// request options (e.g. keep_alive) can be added without coupling providers.

import { logger } from '../runtime/otel.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';
import type { AgentFunction } from '../types/function.js';
import type { AssistantMessageEvent } from '../types/stream-event.js';
import { loadModel } from './load.js';
import {
  buildFinal,
  classifyLmstudioError,
  emptyPartial,
  handleChunk,
  isLoadFailureMessage,
  syntheticErrorEvent,
} from './sse.js';
import type { ChatCompletionsConfig } from './types.js';
import { toOpenaiMessages } from './wire-messages.js';
import { functionsToOpenai } from './wire-tools.js';

/** Catalog placeholder id; routes to "whatever LM Studio currently has loaded". */
export const PLACEHOLDER_MODEL_ID = 'lmstudio-local';

/** Connect + first-byte timeout for any LM Studio HTTP call. */
const FETCH_TIMEOUT_MS = 30_000;

export type StreamArgs = {
  cfg: ChatCompletionsConfig;
  system_prompt: string;
  messages: AgentMessage[];
  tools: AgentFunction[];
};

/**
 * Run `fetch` with an AbortController-based timeout. Without this a
 * misconfigured URL (typo, dead LAN host) hangs for ~75s (macOS SYN
 * timeout) and the UI stays stuck on "thinking…" the whole time.
 */
async function fetchWithTimeout(url: string, init: RequestInit, timeoutMs: number): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Build the chat-completions URL's sibling `/v1/models` endpoint by
 * trimming the trailing `/chat/completions` (if present). Works for the
 * default `…/v1/chat/completions` and for any user-supplied URL that
 * follows the same pattern.
 */
function modelsUrl(chatUrl: string): string {
  const trimmed = chatUrl.replace(/\/chat\/completions\/?$/, '');
  return `${trimmed}/models`;
}

/**
 * When the request comes in with the catalog placeholder `lmstudio-local`,
 * ask LM Studio which model is actually loaded and substitute that id. If
 * discovery fails (network error, empty list, malformed response), return
 * the placeholder unchanged — the downstream fetch will then surface a
 * clear error rather than this helper silently masking one.
 *
 * For explicit model ids we forward verbatim. If the model isn't currently
 * loaded, either LM Studio's JIT setting auto-loads it, or the caller can
 * pre-load it via the `provider::lmstudio::load_model` bus function.
 */
async function resolveModel(cfg: ChatCompletionsConfig, headers: Record<string, string>): Promise<string> {
  if (cfg.model !== PLACEHOLDER_MODEL_ID) return cfg.model;
  try {
    const resp = await fetchWithTimeout(
      modelsUrl(cfg.url),
      { method: 'GET', headers },
      5_000,
    );
    if (!resp.ok) {
      logger.warn('lmstudio /v1/models returned non-2xx; using placeholder', {
        status: resp.status,
      });
      return cfg.model;
    }
    const parsed = (await resp.json()) as { data?: Array<{ id?: string }> };
    const first = parsed.data?.find((m) => typeof m?.id === 'string' && (m.id as string).length > 0)?.id;
    if (first) {
      logger.info('lmstudio: resolved placeholder to loaded model', { model: first });
      return first;
    }
    logger.warn('lmstudio /v1/models returned empty data; using placeholder', {});
    return cfg.model;
  } catch (err) {
    logger.warn('lmstudio /v1/models discovery failed; using placeholder', { err: String(err) });
    return cfg.model;
  }
}

/**
 * One attempt at the chat-completions request + SSE drain. Yields the
 * normal stream events. Pulled out of `streamLmstudio` so the outer
 * generator can call it twice when the first attempt fails with a
 * load-failure shape — auto-loading the model in between.
 */
async function* attemptStream(
  cfg: ChatCompletionsConfig,
  headers: Record<string, string>,
  effectiveModel: string,
  system_prompt: string,
  messages: AgentMessage[],
  tools: AgentFunction[],
): AsyncGenerator<AssistantMessageEvent> {
  const body: Record<string, unknown> = {
    model: effectiveModel,
    max_completion_tokens: cfg.max_tokens,
    messages: toOpenaiMessages(messages, system_prompt),
    stream: true,
    stream_options: { include_usage: true },
  };
  if (tools.length > 0) body.tools = functionsToOpenai(tools);

  let resp: Response;
  try {
    resp = await fetchWithTimeout(
      cfg.url,
      { method: 'POST', headers, body: JSON.stringify(body) },
      FETCH_TIMEOUT_MS,
    );
  } catch (err) {
    const msg =
      err instanceof Error && err.name === 'AbortError'
        ? `lmstudio fetch timed out after ${FETCH_TIMEOUT_MS / 1000}s (is ${cfg.url} reachable?)`
        : `lmstudio fetch failed: ${String(err)}`;
    yield syntheticErrorEvent(msg, effectiveModel, cfg.provider_name);
    return;
  }
  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    // Truncate + strip control chars before surfacing — non-2xx
    // responses can carry HTML auth-redirect bodies, proxy error
    // pages, or attacker-controlled content. Length cap keeps the
    // message readable; control-char strip avoids ANSI / newline
    // injection into log lines and live regions.
    const safeText = text
      // eslint-disable-next-line no-control-regex
      .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')
      .slice(0, 256);
    yield syntheticErrorEvent(
      safeText || `lmstudio http ${resp.status}`,
      cfg.model,
      cfg.provider_name,
      classifyLmstudioError(safeText, resp.status),
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
    yield syntheticErrorEvent('lmstudio response missing body', effectiveModel, cfg.provider_name);
    return;
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  // Counts successfully-parsed SSE data chunks. If we reach EOF with zero,
  // LM Studio returned a 200 with a non-SSE body (e.g. an HTML dashboard
  // page when no model is loaded on some builds) — surface that as an
  // explicit error rather than silently emitting an empty `done`.
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
          yield { type: 'done', message: buildFinal(state, effectiveModel, cfg.provider_name) };
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
            // handleChunk returns an `error` event when LM Studio sent an
            // SSE error chunk (template render failure, mid-stream OOM,
            // model unloaded). Stop reading immediately so the rest of
            // the body (typically just a connection close) doesn't fall
            // into the EOF-without-finish_reason guard and overwrite the
            // server's specific error message.
            if (e.type === 'error') return;
          }
        }
      }
    }
  } catch (err) {
    logger.warn('lmstudio stream read failed', { err: String(err) });
    yield syntheticErrorEvent(`stream read failed: ${String(err)}`, effectiveModel, cfg.provider_name);
    return;
  }
  if (parsedChunkCount === 0) {
    // Phrased to avoid matching `isLoadFailureMessage` — this guard
    // usually means a wrong URL (LM Studio's HTML dashboard) and an
    // auto-load retry would not help. We keep the diagnostic text but
    // describe the cause as "non-SSE body" rather than mentioning
    // "no model is loaded".
    yield syntheticErrorEvent(
      'lmstudio returned a 200 response with a non-SSE body (no parseable chunks). The endpoint URL is likely wrong, or LM Studio served its dashboard page in place of an SSE stream.',
      cfg.model,
      cfg.provider_name,
    );
    return;
  }
  // We parsed some chunks but never saw `data: [DONE]` AND never saw a
  // `finish_reason` in any chunk. That means LM Studio closed the body
  // mid-response — typical causes: GPU OOM during generation, model
  // unloaded mid-stream, host connection dropped, or context exhausted
  // by the running output. Without this guard the user sees a silently
  // truncated reply with no error, because state.stop_reason is still
  // its default 'end'. Promote to an explicit error so the UI can show
  // "stream closed mid-response" instead of pretending success.
  if (!state.saw_finish_reason) {
    const partial = buildFinal(state, effectiveModel, cfg.provider_name);
    const tokenHint =
      partial.usage && (partial.usage.output ?? 0) > 0
        ? ` after ~${partial.usage.output} output tokens`
        : '';
    yield syntheticErrorEvent(
      `lmstudio stream closed mid-response${tokenHint} — the server ended the SSE body without a [DONE] marker or finish_reason. Common causes: model unloaded, GPU OOM, host disconnect, or context exhausted during generation.`,
      effectiveModel,
      cfg.provider_name,
      'transient',
    );
    return;
  }
  yield { type: 'done', message: buildFinal(state, effectiveModel, cfg.provider_name) };
}

export async function* streamLmstudio({
  cfg,
  system_prompt,
  messages,
  tools,
}: StreamArgs): AsyncGenerator<AssistantMessageEvent> {
  const authName = cfg.auth_header_name ?? 'Authorization';
  const authPrefix = cfg.auth_value_prefix ?? 'Bearer ';
  const headers: Record<string, string> = {
    'content-type': 'application/json',
    [authName]: `${authPrefix}${cfg.api_key}`,
  };
  for (const [k, v] of cfg.extra_headers ?? []) headers[k] = v;

  // Resolve `lmstudio-local` → the actual loaded model id, so the catalog
  // placeholder "just works" without the caller having to know what's loaded.
  const effectiveModel = await resolveModel(cfg, headers);

  // Auto-load retry: on the first attempt, if the response is a
  // load-failure error AND we have not yet streamed any real progress
  // (text, tool, or thinking deltas) to the user, swallow the failure,
  // call `provider::lmstudio::load_model` for the resolved model id,
  // then retry the chat completion exactly once. This converts the
  // common "user picked a model that's listed but not actually loaded"
  // failure into a working call. The retry guard ensures we never loop:
  // if the second attempt fails too, the error reaches the caller.
  let retried = false;
  for (;;) {
    // Buffer the first `start` event so we can DROP it (rather than
    // surface a confusing "started, then errored" pair) when we decide
    // to retry. Any non-start event flips `committed` to true and we
    // stream straight through from that point on.
    let bufferedStart: AssistantMessageEvent | null = null;
    let committed = false;
    let retryRequested = false;

    for await (const ev of attemptStream(
      cfg,
      headers,
      effectiveModel,
      system_prompt,
      messages,
      tools,
    )) {
      if (!committed) {
        if (ev.type === 'start') {
          bufferedStart = ev;
          continue;
        }
        if (
          ev.type === 'error' &&
          !retried &&
          isLoadFailureMessage(ev.error.error_message ?? '')
        ) {
          retryRequested = true;
          break;
        }
        // First non-start, non-retriable event — flush the buffered start
        // so the downstream sees the canonical `start → … → done|error`
        // sequence.
        if (bufferedStart) {
          yield bufferedStart;
          bufferedStart = null;
        }
        committed = true;
      }
      yield ev;
    }

    if (retryRequested) {
      logger.info('lmstudio: auto-load retry triggered by load-failure error', {
        model: effectiveModel,
      });
      try {
        await loadModel(cfg.url, headers, effectiveModel);
      } catch (loadErr) {
        // The auto-load itself failed (404 on older LM Studio without the
        // native v1 endpoints, model that won't load on this hardware,
        // etc.). Surface as transient so the operator/UI knows it can
        // potentially be retried after fixing the environment.
        yield syntheticErrorEvent(
          `auto-load failed for "${effectiveModel}": ${String(loadErr)}`,
          effectiveModel,
          cfg.provider_name,
          'transient',
        );
        return;
      }
      retried = true;
      continue;
    }

    // Inner finished naturally (committed events + done/error already
    // forwarded). If by some path it only emitted `start`, flush it so
    // we never leave the downstream wondering.
    if (bufferedStart && !committed) yield bufferedStart;
    return;
  }
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
      model: 'lmstudio',
      provider: 'lmstudio',
      timestamp: Date.now(),
    }
  );
}
