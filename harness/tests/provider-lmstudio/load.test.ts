import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  loadModel,
  nativeLoadUrl,
  nativeUnloadUrl,
  unloadModel,
} from '../../src/provider-lmstudio/load.js';

describe('nativeLoadUrl', () => {
  it('rewrites /v1/chat/completions to /api/v1/models/load', () => {
    expect(nativeLoadUrl('http://localhost:1234/v1/chat/completions')).toBe(
      'http://localhost:1234/api/v1/models/load',
    );
  });

  it('rewrites /api/v0/chat/completions to /api/v1/models/load', () => {
    expect(nativeLoadUrl('http://localhost:1234/api/v0/chat/completions')).toBe(
      'http://localhost:1234/api/v1/models/load',
    );
  });
});

describe('nativeUnloadUrl', () => {
  it('rewrites /v1/chat/completions to /api/v1/models/unload', () => {
    expect(nativeUnloadUrl('http://localhost:1234/v1/chat/completions')).toBe(
      'http://localhost:1234/api/v1/models/unload',
    );
  });
});

describe('loadModel', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('POSTs to /api/v1/models/load and returns the load result', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          type: 'llm',
          instance_id: 'qwen/qwen3-4b-2507',
          load_time_seconds: 7.42,
          status: 'loaded',
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    globalThis.fetch = fetchMock as typeof globalThis.fetch;

    const out = await loadModel(
      'http://localhost:1234/v1/chat/completions',
      { Authorization: 'Bearer lm-studio' },
      'qwen/qwen3-4b-2507',
      { context_length: 8192, flash_attention: true },
    );

    expect(out.status).toBe('loaded');
    expect(out.instance_id).toBe('qwen/qwen3-4b-2507');
    expect(out.load_time_seconds).toBe(7.42);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://localhost:1234/api/v1/models/load');
    expect(init.method).toBe('POST');
    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(body.model).toBe('qwen/qwen3-4b-2507');
    expect(body.context_length).toBe(8192);
    expect(body.flash_attention).toBe(true);
  });

  it('throws with the LM Studio error body on non-2xx status', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        new Response('model not downloaded', { status: 404 }),
      ) as typeof globalThis.fetch;

    await expect(
      loadModel(
        'http://localhost:1234/v1/chat/completions',
        {},
        'missing/model',
      ),
    ).rejects.toThrow(/lmstudio load failed \(404\).*model not downloaded/);
  });

  it('attaches a 404-specific hint pointing at LM Studio 0.4+ requirement', async () => {
    // Older LM Studio (pre-0.4) returns 404 on the native v1 endpoint.
    // Operators need to know that, not just "404 from somewhere".
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(new Response('Not Found', { status: 404 })) as typeof globalThis.fetch;

    await expect(
      loadModel('http://localhost:1234/v1/chat/completions', {}, 'm'),
    ).rejects.toThrow(/LM Studio 0\.4\+ only/i);
  });

  it('attaches a generic common-causes hint on 500 responses', async () => {
    // Regression: the QA report against zai-org/glm-4.7-flash returned a
    // 500 with `{"error":{"type":"model_load_failed",...}}`. The user
    // needs a checklist (downloaded, template, VRAM, quantization), not
    // a bare "Failed to load model." pass-through.
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        new Response(
          JSON.stringify({
            error: { type: 'model_load_failed', message: 'Failed to load model.' },
          }),
          { status: 500 },
        ),
      ) as typeof globalThis.fetch;

    await expect(
      loadModel('http://localhost:1234/v1/chat/completions', {}, 'zai-org/glm-4.7-flash'),
    ).rejects.toThrow(/common causes:.*VRAM.*quantization.*Developer.*panel/is);
  });

  it('throws a timeout error when the load fetch aborts', async () => {
    globalThis.fetch = vi.fn((_url, init) => {
      const signal = (init as RequestInit).signal;
      return new Promise<Response>((_resolve, reject) => {
        signal?.addEventListener('abort', () => {
          const err = new Error('aborted');
          (err as Error & { name: string }).name = 'AbortError';
          reject(err);
        });
      });
    }) as typeof globalThis.fetch;

    // Trim the timeout to keep the test fast — the actual load timeout
    // would be 120s; we just need to prove the abort path surfaces a
    // clear timeout error rather than the raw AbortError.
    vi.useFakeTimers();
    const promise = loadModel(
      'http://localhost:1234/v1/chat/completions',
      {},
      'huge/model',
    );
    await vi.advanceTimersByTimeAsync(120_001);
    await expect(promise).rejects.toThrow(/timed out/i);
    vi.useRealTimers();
  });

  it('propagates non-abort transport errors verbatim', async () => {
    globalThis.fetch = vi
      .fn()
      .mockRejectedValue(new Error('ECONNREFUSED 127.0.0.1:1234')) as typeof globalThis.fetch;
    await expect(
      loadModel('http://localhost:1234/v1/chat/completions', {}, 'm'),
    ).rejects.toThrow(/ECONNREFUSED/);
  });
});

describe('unloadModel', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('POSTs to /api/v1/models/unload with the instance_id and returns the response', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({ instance_id: 'qwen/qwen3-4b-2507' }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    globalThis.fetch = fetchMock as typeof globalThis.fetch;

    const out = await unloadModel(
      'http://localhost:1234/v1/chat/completions',
      {},
      'qwen/qwen3-4b-2507',
    );
    expect(out.instance_id).toBe('qwen/qwen3-4b-2507');

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://localhost:1234/api/v1/models/unload');
    expect(init.method).toBe('POST');
    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(body.instance_id).toBe('qwen/qwen3-4b-2507');
  });

  it('throws when LM Studio returns a non-2xx response', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(
        new Response('not loaded', { status: 400 }),
      ) as typeof globalThis.fetch;
    await expect(
      unloadModel('http://localhost:1234/v1/chat/completions', {}, 'm'),
    ).rejects.toThrow(/lmstudio unload failed \(400\)/);
  });
});
