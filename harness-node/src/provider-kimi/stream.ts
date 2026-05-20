// Kept separate from provider-openai so Moonshot-specific request options
// can be added without coupling the two providers.

import { logger } from '../runtime/otel.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';
import type { AgentFunction } from '../types/function.js';
import type { AssistantMessageEvent } from '../types/stream-event.js';
import {
  buildFinal,
  classifyKimiError,
  emptyPartial,
  handleChunk,
  syntheticErrorEvent,
} from './sse.js';
import type { ChatCompletionsConfig } from './types.js';
import { toOpenaiMessages } from './wire-messages.js';
import { functionsToOpenai } from './wire-tools.js';

export type StreamArgs = {
  cfg: ChatCompletionsConfig;
  system_prompt: string;
  messages: AgentMessage[];
  tools: AgentFunction[];
};

export async function* streamKimi({
  cfg,
  system_prompt,
  messages,
  tools,
}: StreamArgs): AsyncGenerator<AssistantMessageEvent> {
  const body: Record<string, unknown> = {
    model: cfg.model,
    max_completion_tokens: cfg.max_tokens,
    messages: toOpenaiMessages(messages, system_prompt),
    stream: true,
    stream_options: { include_usage: true },
  };
  if (tools.length > 0) body.tools = functionsToOpenai(tools);

  const authName = cfg.auth_header_name ?? 'Authorization';
  const authPrefix = cfg.auth_value_prefix ?? 'Bearer ';
  const headers: Record<string, string> = {
    'content-type': 'application/json',
    [authName]: `${authPrefix}${cfg.api_key}`,
  };
  for (const [k, v] of cfg.extra_headers ?? []) headers[k] = v;

  let resp: Response;
  try {
    resp = await fetch(cfg.url, {
      method: 'POST',
      headers,
      body: JSON.stringify(body),
    });
  } catch (err) {
    yield syntheticErrorEvent(`kimi fetch failed: ${String(err)}`, cfg.model, cfg.provider_name);
    return;
  }
  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    yield syntheticErrorEvent(
      text || `kimi http ${resp.status}`,
      cfg.model,
      cfg.provider_name,
      classifyKimiError(text, resp.status),
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
    model: cfg.model,
    provider: cfg.provider_name,
    timestamp: Date.now(),
  };
  yield { type: 'start', partial };

  const state = emptyPartial();
  if (!resp.body) {
    yield syntheticErrorEvent('kimi response missing body', cfg.model, cfg.provider_name);
    return;
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
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
          yield { type: 'done', message: buildFinal(state, cfg.model, cfg.provider_name) };
          return;
        }
        let parsed: Record<string, unknown> | null = null;
        try {
          parsed = JSON.parse(dataLine) as Record<string, unknown>;
        } catch {
          continue;
        }
        if (parsed) {
          for (const e of handleChunk(parsed, state, cfg.model, cfg.provider_name)) yield e;
        }
      }
    }
  } catch (err) {
    logger.warn('kimi stream read failed', { err: String(err) });
    yield syntheticErrorEvent(`stream read failed: ${String(err)}`, cfg.model, cfg.provider_name);
    return;
  }
  yield { type: 'done', message: buildFinal(state, cfg.model, cfg.provider_name) };
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
      model: 'kimi',
      provider: 'kimi',
      timestamp: Date.now(),
    }
  );
}
