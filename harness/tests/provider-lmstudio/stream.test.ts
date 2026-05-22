import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { collect, streamLmstudio } from '../../src/provider-lmstudio/stream.js';
import type { ChatCompletionsConfig } from '../../src/provider-lmstudio/types.js';

const cfg: ChatCompletionsConfig = {
  url: 'http://localhost:1234/v1/chat/completions',
  provider_name: 'lmstudio',
  model: 'qwen/qwen3-4b-2507',
  api_key: 'lm-studio',
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

describe('streamLmstudio', () => {
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
    for await (const ev of streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] })) {
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

  it('classifies HTTP 502 (LM Studio not running) as transient', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(errorResponse(502, 'connection refused upstream'));
    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_kind).toBe('transient');
  });

  it('classifies "no model loaded" 4xx response as transient', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(errorResponse(404, 'no model is loaded; please load a model first'));
    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.error_kind).toBe('transient');
  });

  it('surfaces fetch transport failures (LM Studio offline) as a single error event', async () => {
    globalThis.fetch = vi.fn().mockRejectedValue(new Error('ECONNREFUSED 127.0.0.1:1234'));
    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toContain('lmstudio fetch failed');
  });

  it('sends Bearer token and JSON body to the configured localhost URL', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(
        sseResponse([
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]),
      );
    globalThis.fetch = fetchMock;

    await collect(streamLmstudio({ cfg, system_prompt: 'sys', messages: [], tools: [] }));

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(cfg.url);
    const headers = init.headers as Record<string, string>;
    expect(headers.Authorization).toBe('Bearer lm-studio');
    expect(headers['content-type']).toBe('application/json');
    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(body.model).toBe(cfg.model);
    expect(body.stream).toBe(true);
    expect((body.messages as Array<{ role: string }>)[0]?.role).toBe('system');
  });

  it('emits an explicit error when LM Studio returns a 200 with a non-SSE body', async () => {
    // LM Studio's UI dashboard at the wrong endpoint, or some builds when no
    // model is loaded, return 200 + HTML. The stream must NOT silently
    // succeed with an empty assistant message — emit a clear error event.
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        new Response('<html><body>LM Studio dashboard</body></html>', {
          status: 200,
          headers: { 'content-type': 'text/html' },
        }),
      );
    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toMatch(/non-SSE body/i);
    expect(final.error_kind).toBe('transient');
  });

  it('emits an explicit error when the stream closes mid-response without a finish_reason or [DONE]', async () => {
    // This is the silent-truncation bug we're guarding against: LM Studio
    // sometimes ends the SSE body abruptly (GPU OOM, model unloaded,
    // host disconnect, context exhausted during generation). Before the
    // fix the stream silently emitted `done` with stop_reason='end' —
    // the user saw a half-written reply and zero error indication. The
    // fix promotes this to an explicit error event so the UI can show a
    // clear "stream closed mid-response" notice.
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        sseResponse([
          'data: {"choices":[{"delta":{"content":"partial"}}]}\n\n',
          // No [DONE], no finish_reason — body just ends here.
        ]),
      );
    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_kind).toBe('transient');
    expect(final.error_message).toMatch(/stream closed mid-response/i);
  });

  it('surfaces an SSE error chunk verbatim instead of the generic "stream closed" message', async () => {
    // LM Studio commits to HTTP 200 + SSE, then on a prompt-template
    // render failure (e.g. qwen3 "No user query found") sends a single
    // chunk shaped `data: {"error":{"message":"..."}}` and closes.
    // Without the fix that chunk's error field is silently ignored and
    // the EOF guard reports the generic "stream closed mid-response".
    // With the fix we surface LM Studio's specific message instead.
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        sseResponse([
          'data: {"error":{"message":"Error rendering prompt with jinja template: \\"No user query found in messages.\\"","type":"invalid_request_error"}}\n\n',
        ]),
      );
    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toMatch(/No user query found in messages/);
    // CRITICAL: must NOT be the generic fallback message.
    expect(final.error_message).not.toMatch(/stream closed mid-response/i);
  });

  it('still emits a clean `done` when the stream sends finish_reason but no trailing [DONE]', async () => {
    // Some servers omit the trailing `data: [DONE]\n\n` line but DO send
    // a chunk with finish_reason. That's a legitimate end — we should
    // NOT promote it to an error; the server explicitly said "stop".
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        sseResponse([
          'data: {"choices":[{"delta":{"content":"hi"}}]}\n\n',
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          // No [DONE] sentinel after — body just ends.
        ]),
      );
    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('end');
    expect(final.error_message).toBeNull();
  });

  it('aborts the fetch after the configured timeout when the URL is unreachable', async () => {
    // Simulate fetch hanging indefinitely — the AbortController should
    // fire and we should emit a clear timeout error, not hang the worker.
    let abortedFromController = false;
    globalThis.fetch = vi.fn((_url, init) => {
      const signal = (init as RequestInit | undefined)?.signal;
      return new Promise<Response>((_resolve, reject) => {
        signal?.addEventListener('abort', () => {
          abortedFromController = true;
          const err = new Error('aborted');
          (err as Error & { name: string }).name = 'AbortError';
          reject(err);
        });
      });
    }) as typeof globalThis.fetch;

    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(abortedFromController).toBe(true);
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toMatch(/timed out/i);
  }, 60_000);

  it('resolves placeholder model `lmstudio-local` by querying /v1/models', async () => {
    const calls: string[] = [];
    globalThis.fetch = vi.fn(async (url, init) => {
      calls.push(String(url));
      if (String(url).endsWith('/v1/models')) {
        return new Response(
          JSON.stringify({ data: [{ id: 'qwen/qwen3.6-35b-a3b' }] }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      // Stream call — capture the body to verify the model id was substituted.
      const body = JSON.parse((init as RequestInit).body as string) as { model: string };
      expect(body.model).toBe('qwen/qwen3.6-35b-a3b');
      return sseResponse([
        'data: {"choices":[{"delta":{"content":"hi"}}]}\n\n',
        'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
        'data: [DONE]\n\n',
      ]);
    }) as typeof globalThis.fetch;

    const placeholderCfg = { ...cfg, model: 'lmstudio-local' };
    const final = await collect(
      streamLmstudio({ cfg: placeholderCfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(calls.some((u) => u.endsWith('/v1/models'))).toBe(true);
    expect(final.stop_reason).toBe('end');
    expect(final.model).toBe('qwen/qwen3.6-35b-a3b');
  });

  it('keeps the placeholder model when /v1/models discovery returns empty data', async () => {
    globalThis.fetch = vi.fn(async (url) => {
      if (String(url).endsWith('/v1/models')) {
        return new Response(JSON.stringify({ data: [] }), { status: 200 });
      }
      return sseResponse([
        'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
        'data: [DONE]\n\n',
      ]);
    }) as typeof globalThis.fetch;

    const placeholderCfg = { ...cfg, model: 'lmstudio-local' };
    const final = await collect(
      streamLmstudio({ cfg: placeholderCfg, system_prompt: '', messages: [], tools: [] }),
    );
    // Discovery returned empty list — fall back to the placeholder unchanged.
    expect(final.model).toBe('lmstudio-local');
  });

  it('forwards tool_call argument deltas as functioncall_delta events', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        sseResponse([
          'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"tc1","function":{"name":"shell::ls","arguments":"{\\"p"}}]}}]}\n\n',
          'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ath\\":\\"/tmp\\"}"}}]}}]}\n\n',
          'data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]),
      );

    const seen: string[] = [];
    for await (const ev of streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] })) {
      seen.push(ev.type);
    }
    expect(seen).toContain('functioncall_delta');
    expect(seen[seen.length - 1]).toBe('done');
  });
});

describe('streamLmstudio — auto-load retry on load-failure error', () => {
  // Regression: in the QA report for `zai-org/glm-4.7-flash` on lmstudio,
  // the chat trigger failed with an SSE error chunk:
  //   {"error":{"message":"The model has crashed without additional
  //   information. (Exit code: null)","type":"model_load_failed"}}
  // The fix: detect that shape on the first attempt, call the native
  // `/api/v1/models/load` endpoint to (re)load the model, and retry the
  // chat completion exactly once. These tests pin the new behavior.
  let originalFetch: typeof globalThis.fetch;
  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });
  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  function sse(chunks: string[]): Response {
    const encoder = new TextEncoder();
    return new Response(
      new ReadableStream<Uint8Array>({
        start(controller) {
          for (const c of chunks) controller.enqueue(encoder.encode(c));
          controller.close();
        },
      }),
      { status: 200, headers: { 'content-type': 'text/event-stream' } },
    );
  }

  it('auto-loads then succeeds when first attempt returns 400 "model has crashed"', async () => {
    const urls: string[] = [];
    globalThis.fetch = vi.fn(async (url) => {
      const u = String(url);
      urls.push(u);
      if (u.endsWith('/v1/chat/completions') && urls.filter((x) => x.endsWith('/v1/chat/completions')).length === 1) {
        // First attempt: LM Studio returns the crash error as 400 body.
        return new Response(
          JSON.stringify({
            error: {
              type: 'model_load_failed',
              message:
                "The model has crashed without additional information. (Exit code: null)",
            },
          }),
          { status: 400, headers: { 'content-type': 'application/json' } },
        );
      }
      if (u.endsWith('/api/v1/models/load')) {
        return new Response(
          JSON.stringify({
            type: 'llm',
            instance_id: 'abc-123',
            load_time_seconds: 1.5,
            status: 'loaded',
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      // Second attempt: real streamed answer.
      return sse([
        'data: {"choices":[{"delta":{"content":"v24"}}]}\n\n',
        'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
        'data: [DONE]\n\n',
      ]);
    }) as typeof globalThis.fetch;

    const events: string[] = [];
    let finalText = '';
    for await (const ev of streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] })) {
      events.push(ev.type);
      if (ev.type === 'done') {
        finalText = ev.message.content
          .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
          .map((c) => c.text)
          .join('');
      }
    }
    // Downstream must NOT see the failed first attempt — the buffered
    // `start` from the first try gets dropped, the synthetic error never
    // leaves the provider, and the second attempt's full event sequence
    // is what reaches the consumer.
    expect(events).toEqual(['start', 'text_start', 'text_delta', 'done']);
    expect(finalText).toBe('v24');
    // load was actually called between the two chat attempts
    expect(urls.filter((u) => u.endsWith('/api/v1/models/load'))).toHaveLength(1);
    expect(urls.filter((u) => u.endsWith('/v1/chat/completions'))).toHaveLength(2);
  });

  it('auto-loads then succeeds when first attempt streams an SSE error chunk', async () => {
    // Different code path: HTTP 200 + SSE, then the error arrives as a
    // single data chunk. This is what kimi/qwen also emit on
    // template-render failures.
    let chatCallCount = 0;
    globalThis.fetch = vi.fn(async (url) => {
      const u = String(url);
      if (u.endsWith('/v1/chat/completions')) {
        chatCallCount++;
        if (chatCallCount === 1) {
          return sse([
            'data: {"error":{"message":"The model has crashed without additional information. (Exit code: null)","type":"model_load_failed"}}\n\n',
          ]);
        }
        return sse([
          'data: {"choices":[{"delta":{"content":"ok"}}]}\n\n',
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]);
      }
      // /api/v1/models/load
      return new Response(
        JSON.stringify({
          type: 'llm',
          instance_id: 'abc-123',
          load_time_seconds: 0.5,
          status: 'loaded',
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }) as typeof globalThis.fetch;

    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('end');
    expect(final.error_message).toBeNull();
    expect(chatCallCount).toBe(2);
  });

  it('does NOT retry a second time — surfaces the error if the retry also fails', async () => {
    // Guard against infinite loops: if both attempts fail with the same
    // load-failure shape, the SECOND error is what reaches the consumer.
    let chatCallCount = 0;
    globalThis.fetch = vi.fn(async (url) => {
      const u = String(url);
      if (u.endsWith('/api/v1/models/load')) {
        return new Response(
          JSON.stringify({
            type: 'llm',
            instance_id: 'abc-123',
            load_time_seconds: 0.5,
            status: 'loaded',
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      chatCallCount++;
      return new Response(
        JSON.stringify({
          error: { type: 'model_load_failed', message: 'Failed to load LLM' },
        }),
        { status: 400, headers: { 'content-type': 'application/json' } },
      );
    }) as typeof globalThis.fetch;

    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(chatCallCount).toBe(2);
    expect(final.stop_reason).toBe('error');
    expect(final.error_kind).toBe('transient');
    expect(final.error_message).toContain('LM Studio could not load the model');
    expect(final.error_message).toContain('Failed to load LLM');
  });

  it('does NOT retry on non-load-failure errors (e.g. context_overflow)', async () => {
    let chatCallCount = 0;
    globalThis.fetch = vi.fn(async () => {
      chatCallCount++;
      return new Response(
        JSON.stringify({ error: { message: 'context length exceeded' } }),
        { status: 400, headers: { 'content-type': 'application/json' } },
      );
    }) as typeof globalThis.fetch;

    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(chatCallCount).toBe(1);
    expect(final.error_kind).toBe('context_overflow');
  });

  it('does NOT retry after tokens have already streamed (no mid-stream do-over)', async () => {
    // If LM Studio sent us tokens then the model crashed, retrying would
    // require us to discard partial output — confusing UX. The retry only
    // fires when the first attempt's first non-start event is the error.
    let chatCallCount = 0;
    globalThis.fetch = vi.fn(async (url) => {
      const u = String(url);
      if (u.endsWith('/v1/chat/completions')) {
        chatCallCount++;
        return sse([
          'data: {"choices":[{"delta":{"content":"partial"}}]}\n\n',
          'data: {"error":{"message":"The model has crashed without additional information. (Exit code: null)"}}\n\n',
        ]);
      }
      throw new Error('unexpected url ' + u);
    }) as typeof globalThis.fetch;

    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(chatCallCount).toBe(1);
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toContain('LM Studio could not load the model');
  });

  it('surfaces auto-load failure cleanly if /api/v1/models/load itself fails', async () => {
    // Older LM Studio (pre-0.4) returns 404 on /api/v1/models/load; we
    // shouldn't pretend the chat succeeded.
    let chatCallCount = 0;
    globalThis.fetch = vi.fn(async (url) => {
      const u = String(url);
      if (u.endsWith('/api/v1/models/load')) {
        return new Response('Not Found', { status: 404 });
      }
      chatCallCount++;
      return new Response(
        JSON.stringify({ error: { type: 'model_load_failed', message: 'The model has crashed' } }),
        { status: 400, headers: { 'content-type': 'application/json' } },
      );
    }) as typeof globalThis.fetch;

    const final = await collect(
      streamLmstudio({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(chatCallCount).toBe(1); // never reached the retry chat call
    expect(final.stop_reason).toBe('error');
    expect(final.error_kind).toBe('transient');
    expect(final.error_message).toContain('auto-load failed');
    expect(final.error_message).toContain(cfg.model);
  });
});
