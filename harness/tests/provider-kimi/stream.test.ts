import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { collect, streamKimi } from '../../src/provider-kimi/stream.js';
import type { ChatCompletionsConfig } from '../../src/provider-kimi/types.js';

const cfg: ChatCompletionsConfig = {
  url: 'https://api.moonshot.ai/v1/chat/completions',
  provider_name: 'kimi',
  model: 'kimi-k2-0905-preview',
  api_key: 'sk-test',
  max_tokens: 256,
};

function sseResponse(chunks: string[], status = 200): Response {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const c of chunks) controller.enqueue(encoder.encode(c));
      controller.close();
    },
  });
  return new Response(stream, {
    status,
    headers: { 'content-type': 'text/event-stream' },
  });
}

function errorResponse(status: number, body: string): Response {
  return new Response(body, { status });
}

describe('streamKimi', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('emits start -> text_start -> text_delta+ -> done on a happy-path stream', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        sseResponse([
          'data: {"choices":[{"delta":{"content":"Hel"}}]}\n\n',
          'data: {"choices":[{"delta":{"content":"lo"}}]}\n\n',
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]),
      );

    const events: string[] = [];
    let finalText = '';
    for await (const ev of streamKimi({ cfg, system_prompt: '', messages: [], tools: [] })) {
      events.push(ev.type);
      if (ev.type === 'done') {
        finalText = ev.message.content
          .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
          .map((c) => c.text)
          .join('');
      }
    }
    expect(events).toEqual(['start', 'text_start', 'text_delta', 'text_delta', 'done']);
    expect(finalText).toBe('Hello');
  });

  it('classifies HTTP 401 as auth_expired', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(errorResponse(401, 'unauthorized'));
    const final = await collect(streamKimi({ cfg, system_prompt: '', messages: [], tools: [] }));
    expect(final.stop_reason).toBe('error');
    expect(final.error_kind).toBe('auth_expired');
  });

  it('classifies HTTP 429 as rate_limited', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(errorResponse(429, 'slow down'));
    const final = await collect(streamKimi({ cfg, system_prompt: '', messages: [], tools: [] }));
    expect(final.error_kind).toBe('rate_limited');
  });

  it('classifies HTTP 500 as transient', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(errorResponse(503, 'service unavailable'));
    const final = await collect(streamKimi({ cfg, system_prompt: '', messages: [], tools: [] }));
    expect(final.error_kind).toBe('transient');
  });

  it('surfaces fetch transport failures as a single error event', async () => {
    globalThis.fetch = vi.fn().mockRejectedValue(new Error('ECONNREFUSED'));
    const final = await collect(streamKimi({ cfg, system_prompt: '', messages: [], tools: [] }));
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toContain('kimi fetch failed');
  });

  it('sends Bearer token and JSON body to the configured URL', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(
        sseResponse([
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]),
      );
    globalThis.fetch = fetchMock;

    await collect(streamKimi({ cfg, system_prompt: 'sys', messages: [], tools: [] }));

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(cfg.url);
    const headers = init.headers as Record<string, string>;
    expect(headers.Authorization).toBe('Bearer sk-test');
    expect(headers['content-type']).toBe('application/json');
    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(body.model).toBe(cfg.model);
    expect(body.stream).toBe(true);
    expect((body.messages as Array<{ role: string }>)[0]?.role).toBe('system');
  });
});
