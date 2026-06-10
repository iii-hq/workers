/**
 * `streamAnthropic` — POST `/v1/messages` (stream:true), parse SSE,
 * yield `AssistantMessageEvent`s. Mirrors
 * `provider-anthropic/src/lib.rs::stream` + `stream_inner`.
 */

import { logger } from '../runtime/otel.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';
import type { AgentFunction } from '../types/function.js';
import type { AssistantMessageEvent } from '../types/stream-event.js';
import { invalidateProviderResolveCache } from './auth.js';
import { applyMessagesCacheAnchor, applyToolsCacheControl, buildSystemField } from './cache.js';
import {
  buildFinal,
  classifyAnthropicError,
  emptyPartial,
  handleSseEvent,
  syntheticErrorEvent,
} from './sse.js';
import type { ThinkingConfig } from './thinking.js';
import { type AnthropicConfig, authHeaderFor } from './types.js';
import { toWireMessages } from './wire-messages.js';
import { functionsToWire } from './wire-tools.js';

export type StreamArgs = {
  cfg: AnthropicConfig;
  system_prompt: string;
  messages: AgentMessage[];
  tools: AgentFunction[];
  /** Extended thinking; absent = off. See `thinking.ts`. */
  thinking?: ThinkingConfig;
};

export async function* streamAnthropic({
  cfg,
  system_prompt,
  messages,
  tools,
  thinking,
}: StreamArgs): AsyncGenerator<AssistantMessageEvent> {
  const wire_messages = toWireMessages(messages) as Record<string, unknown>[];
  applyMessagesCacheAnchor(wire_messages);
  const wire_tools = functionsToWire(tools) as Record<string, unknown>[];
  applyToolsCacheControl(wire_tools);
  const body = {
    model: cfg.model,
    max_tokens: cfg.max_tokens,
    ...(thinking ? { thinking } : {}),
    system: buildSystemField(system_prompt),
    messages: wire_messages,
    tools: wire_tools,
    stream: true,
  };
  const [headerName, headerValue] = authHeaderFor(cfg);
  const headers: Record<string, string> = {
    [headerName]: headerValue,
    'anthropic-version': '2023-06-01',
    'content-type': 'application/json',
  };
  if (thinking) {
    headers['anthropic-beta'] = 'interleaved-thinking-2025-05-14';
  }

  const ac = new AbortController();
  let resp: Response;
  try {
    resp = await fetch(cfg.api_url, {
      method: 'POST',
      headers,
      body: JSON.stringify(body),
      signal: ac.signal,
    });
  } catch (err) {
    yield syntheticErrorEvent(`anthropic fetch failed: ${String(err)}`, cfg.model);
    return;
  }

  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    const kind = classifyAnthropicError(text, resp.status);
    if (kind === 'auth_expired') invalidateProviderResolveCache();
    yield syntheticErrorEvent(text || `anthropic http ${resp.status}`, cfg.model, kind);
    return;
  }

  const partialMsg: AssistantMessage = {
    role: 'assistant',
    content: [],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model: cfg.model,
    provider: 'anthropic',
    timestamp: Date.now(),
  };
  yield { type: 'start', partial: partialMsg };

  const state = emptyPartial();
  let buf = '';
  if (!resp.body) {
    yield syntheticErrorEvent('anthropic response missing body', cfg.model);
    return;
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      let idx = buf.indexOf('\n\n');
      while (idx >= 0) {
        const block = buf.slice(0, idx);
        buf = buf.slice(idx + 2);
        const events = handleSseEvent(block, state, cfg.model);
        for (const e of events) yield e;
        idx = buf.indexOf('\n\n');
      }
    }
  } catch (err) {
    logger.warn('anthropic stream read failed', { err: String(err) });
    yield syntheticErrorEvent(`stream read failed: ${String(err)}`, cfg.model);
    return;
  }
  yield { type: 'done', message: buildFinal(state, cfg.model) };
}

/** Drain a stream into the final AssistantMessage. */
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
      model: 'anthropic',
      provider: 'anthropic',
      timestamp: Date.now(),
    }
  );
}
