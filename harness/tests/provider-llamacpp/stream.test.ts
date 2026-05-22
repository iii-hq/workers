import { afterEach, describe, expect, it, vi } from 'vitest';
import { collect, streamLlamacpp } from '../../src/provider-llamacpp/stream.js';
import type { ChatCompletionsConfig } from '../../src/provider-llamacpp/types.js';

const cfg: ChatCompletionsConfig = {
  url: 'http://localhost:8080/v1/chat/completions',
  provider_name: 'llamacpp',
  model: 'Meta-Llama-3.1-8B',
  api_key: '',
  max_tokens: 1024,
};

function sseStream(chunks: string[]): ReadableStream<Uint8Array> {
  const enc = new TextEncoder();
  let i = 0;
  return new ReadableStream({
    pull(c) {
      if (i < chunks.length) c.enqueue(enc.encode(chunks[i++]));
      else c.close();
    },
  });
}

function okResponse(body: ReadableStream<Uint8Array>): Response {
  return new Response(body, {
    status: 200,
    headers: { 'content-type': 'text/event-stream' },
  });
}

const originalFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

describe('streamLlamacpp', () => {
  it('streams a single text response end-to-end', async () => {
    globalThis.fetch = vi.fn(async () =>
      okResponse(
        sseStream([
          'data: {"choices":[{"delta":{"content":"Hel"}}]}\n\n',
          'data: {"choices":[{"delta":{"content":"lo"}}]}\n\n',
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]),
      ),
    ) as typeof globalThis.fetch;
    const final = await collect(
      streamLlamacpp({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('end');
    const text = final.content.find((c) => c.type === 'text');
    if (text?.type !== 'text') throw new Error('missing text block');
    expect(text.text).toBe('Hello');
  });

  it('omits Authorization when api_key is empty', async () => {
    let observedAuth: string | undefined;
    globalThis.fetch = vi.fn(async (_url, init) => {
      observedAuth = (init as RequestInit | undefined)?.headers
        ? (init as { headers: Record<string, string> }).headers.Authorization
        : undefined;
      return okResponse(
        sseStream([
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]),
      );
    }) as typeof globalThis.fetch;
    await collect(streamLlamacpp({ cfg, system_prompt: '', messages: [], tools: [] }));
    expect(observedAuth).toBeUndefined();
  });

  it('emits Authorization when api_key is set', async () => {
    let observedAuth: string | undefined;
    globalThis.fetch = vi.fn(async (_url, init) => {
      observedAuth = (init as { headers: Record<string, string> }).headers.Authorization;
      return okResponse(
        sseStream([
          'data: {"choices":[{"finish_reason":"stop","delta":{}}]}\n\n',
          'data: [DONE]\n\n',
        ]),
      );
    }) as typeof globalThis.fetch;
    await collect(
      streamLlamacpp({
        cfg: { ...cfg, api_key: 'sk-real' },
        system_prompt: '',
        messages: [],
        tools: [],
      }),
    );
    expect(observedAuth).toBe('Bearer sk-real');
  });

  it('surfaces a clear error when llama-server returns a non-2xx body (truncated)', async () => {
    globalThis.fetch = vi.fn(
      async () =>
        new Response(`<html>${'x'.repeat(2000)}</html>`, {
          status: 500,
          headers: { 'content-type': 'text/html' },
        }),
    ) as typeof globalThis.fetch;
    const final = await collect(
      streamLlamacpp({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_kind).toBe('transient');
    // Length-capped to ~256 bytes (the truncation guard).
    expect((final.error_message ?? '').length).toBeLessThanOrEqual(256);
  });

  it('surfaces a stop-mid-stream error when SSE closes without [DONE] or finish_reason', async () => {
    globalThis.fetch = vi.fn(async () =>
      okResponse(
        sseStream([
          'data: {"choices":[{"delta":{"content":"partial"}}]}\n\n',
          // server closes the body here — no [DONE], no finish_reason
        ]),
      ),
    ) as typeof globalThis.fetch;
    const final = await collect(
      streamLlamacpp({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toMatch(/stream closed mid-response/);
  });

  it('surfaces a clear error when a 200 response has no SSE chunks (wrong URL / HTML page)', async () => {
    globalThis.fetch = vi.fn(
      async () =>
        new Response('<html>dashboard</html>', {
          status: 200,
          headers: { 'content-type': 'text/html' },
        }),
    ) as typeof globalThis.fetch;
    const final = await collect(
      streamLlamacpp({ cfg, system_prompt: '', messages: [], tools: [] }),
    );
    expect(final.stop_reason).toBe('error');
    expect(final.error_message).toMatch(/non-SSE body/);
  });

  it('aborts the fetch after the configured timeout when the URL is unreachable', async () => {
    // Pin the timeout via env so the fake-timer advance is precise;
    // resolveFetchTimeoutMs is read fresh inside streamLlamacpp.
    const prev = process.env.LLAMACPP_FETCH_TIMEOUT_MS;
    process.env.LLAMACPP_FETCH_TIMEOUT_MS = '30000';
    vi.useFakeTimers();
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
    try {
      const final$ = collect(streamLlamacpp({ cfg, system_prompt: '', messages: [], tools: [] }));
      await vi.advanceTimersByTimeAsync(30_001);
      const final = await final$;
      expect(abortedFromController).toBe(true);
      expect(final.stop_reason).toBe('error');
      expect(final.error_message).toMatch(/timed out after 30s/i);
    } finally {
      vi.useRealTimers();
      if (prev === undefined) delete process.env.LLAMACPP_FETCH_TIMEOUT_MS;
      else process.env.LLAMACPP_FETCH_TIMEOUT_MS = prev;
    }
  });

  it('honours LLAMACPP_FETCH_TIMEOUT_MS for the abort message', async () => {
    const prev = process.env.LLAMACPP_FETCH_TIMEOUT_MS;
    process.env.LLAMACPP_FETCH_TIMEOUT_MS = '60000';
    vi.useFakeTimers();
    globalThis.fetch = vi.fn((_url, init) => {
      const signal = (init as RequestInit | undefined)?.signal;
      return new Promise<Response>((_resolve, reject) => {
        signal?.addEventListener('abort', () => {
          const err = new Error('aborted');
          (err as Error & { name: string }).name = 'AbortError';
          reject(err);
        });
      });
    }) as typeof globalThis.fetch;
    try {
      const final$ = collect(streamLlamacpp({ cfg, system_prompt: '', messages: [], tools: [] }));
      await vi.advanceTimersByTimeAsync(60_001);
      const final = await final$;
      expect(final.error_message).toMatch(/timed out after 60s/i);
    } finally {
      vi.useRealTimers();
      if (prev === undefined) delete process.env.LLAMACPP_FETCH_TIMEOUT_MS;
      else process.env.LLAMACPP_FETCH_TIMEOUT_MS = prev;
    }
  });

  it('falls back to the 120s default when LLAMACPP_FETCH_TIMEOUT_MS is unset', async () => {
    // resolveFetchTimeoutMs is the unit; verify the contract here so a
    // future default change is caught.
    const prev = process.env.LLAMACPP_FETCH_TIMEOUT_MS;
    delete process.env.LLAMACPP_FETCH_TIMEOUT_MS;
    try {
      const { resolveFetchTimeoutMs } = await import('../../src/provider-llamacpp/stream.js');
      expect(resolveFetchTimeoutMs()).toBe(120_000);
      process.env.LLAMACPP_FETCH_TIMEOUT_MS = 'not-a-number';
      expect(resolveFetchTimeoutMs()).toBe(120_000);
      process.env.LLAMACPP_FETCH_TIMEOUT_MS = '500'; // below floor
      expect(resolveFetchTimeoutMs()).toBe(120_000);
      process.env.LLAMACPP_FETCH_TIMEOUT_MS = '45000';
      expect(resolveFetchTimeoutMs()).toBe(45_000);
    } finally {
      if (prev === undefined) delete process.env.LLAMACPP_FETCH_TIMEOUT_MS;
      else process.env.LLAMACPP_FETCH_TIMEOUT_MS = prev;
    }
  });
});
