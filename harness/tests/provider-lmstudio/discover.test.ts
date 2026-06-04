import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  discoverAllDownloadedModels,
  discoverAndRegister,
  discoverLoadedIds,
  discoverLoadedModels,
  nativeModelsUrl,
  registerDiscovered,
  toCatalogModel,
} from '../../src/provider-lmstudio/discover.js';
import type { Model } from '../../src/models-catalog/types.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('nativeModelsUrl', () => {
  it('rewrites the OpenAI-compatible /v1/chat/completions path to /api/v0/models', () => {
    expect(nativeModelsUrl('http://localhost:1234/v1/chat/completions')).toBe(
      'http://localhost:1234/api/v0/models',
    );
  });

  it('rewrites the native /api/v0/chat/completions path to /api/v0/models', () => {
    expect(nativeModelsUrl('http://localhost:1234/api/v0/chat/completions')).toBe(
      'http://localhost:1234/api/v0/models',
    );
  });

  it('tolerates a trailing slash on the chat-completions path', () => {
    expect(nativeModelsUrl('http://lan:1234/v1/chat/completions/')).toBe(
      'http://lan:1234/api/v0/models',
    );
  });

  it('handles a future /api/v2/chat/completions path', () => {
    // Forward-compat for whenever LM Studio bumps the native version.
    expect(nativeModelsUrl('http://h:1234/api/v2/chat/completions')).toBe(
      'http://h:1234/api/v0/models',
    );
  });
});

describe('toCatalogModel', () => {
  it('maps a typical loaded LLM entry to the iii catalog Model shape', () => {
    const out = toCatalogModel({
      id: 'qwen/qwen3-4b-2507',
      type: 'llm',
      state: 'loaded',
      arch: 'qwen3',
      quantization: 'Q4_K_M',
      max_context_length: 32768,
      loaded_context_length: 8192,
    });
    expect(out).toEqual({
      id: 'qwen/qwen3-4b-2507',
      provider: 'lmstudio',
      api: 'openai-completions',
      display_name: 'qwen/qwen3-4b-2507',
      context_window: 8192, // prefers loaded over max
      max_output_tokens: 8192,
      supports_thinking: false,
      supports_xhigh: false,
      supports_tools: true,
      supports_vision: false,
      supports_cache: false,
      transports: ['sse'],
    });
  });

  it('marks vision-language entries as supports_vision: true', () => {
    const out = toCatalogModel({ id: 'a/b', type: 'vlm', state: 'loaded' });
    expect(out?.supports_vision).toBe(true);
  });

  it('returns null for embeddings models so we never route chat to them', () => {
    expect(toCatalogModel({ id: 'nomic-embed', type: 'embeddings', state: 'loaded' })).toBeNull();
  });

  it('returns null for entries missing an id', () => {
    expect(toCatalogModel({ type: 'llm', state: 'loaded' })).toBeNull();
  });

  it('falls back to max_context_length when loaded_context_length is missing', () => {
    const out = toCatalogModel({
      id: 'm',
      type: 'llm',
      state: 'loaded',
      max_context_length: 16384,
    });
    expect(out?.context_window).toBe(16384);
  });

  it('falls back to the default context window when both lengths are missing', () => {
    const out = toCatalogModel({ id: 'm', type: 'llm', state: 'loaded' });
    expect(out?.context_window).toBe(32768);
  });

  it('accepts entries with no `type` field (older LM Studio builds)', () => {
    const out = toCatalogModel({ id: 'legacy/model', state: 'loaded' });
    expect(out?.id).toBe('legacy/model');
  });
});

describe('discoverLoadedModels', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('returns the catalog Model for each loaded LLM, skipping unloaded and non-llm entries', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          data: [
            { id: 'a/llm-loaded', type: 'llm', state: 'loaded', max_context_length: 4096 },
            { id: 'b/llm-not-loaded', type: 'llm', state: 'not-loaded', max_context_length: 4096 },
            { id: 'c/embed', type: 'embeddings', state: 'loaded' },
            { id: 'd/vlm-loaded', type: 'vlm', state: 'loaded', loaded_context_length: 2048 },
          ],
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    ) as typeof globalThis.fetch;

    const out = await discoverLoadedModels('http://localhost:1234/v1/chat/completions', {
      Authorization: 'Bearer lm-studio',
    });
    expect(out.map((m) => m.id)).toEqual(['a/llm-loaded', 'd/vlm-loaded']);
    expect(out[1]?.supports_vision).toBe(true);
  });

  it('returns [] when LM Studio is unreachable (fetch throws)', async () => {
    globalThis.fetch = vi
      .fn()
      .mockRejectedValue(new Error('ECONNREFUSED 127.0.0.1:1234')) as typeof globalThis.fetch;
    const out = await discoverLoadedModels('http://localhost:1234/v1/chat/completions', {});
    expect(out).toEqual([]);
  });

  it('returns [] when LM Studio returns a non-2xx status', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(new Response('nope', { status: 500 })) as typeof globalThis.fetch;
    const out = await discoverLoadedModels('http://localhost:1234/v1/chat/completions', {});
    expect(out).toEqual([]);
  });

  it('returns [] when LM Studio returns a 200 with invalid JSON', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response('<html>dashboard</html>', {
        status: 200,
        headers: { 'content-type': 'text/html' },
      }),
    ) as typeof globalThis.fetch;
    const out = await discoverLoadedModels('http://localhost:1234/v1/chat/completions', {});
    expect(out).toEqual([]);
  });

  it('hits the /api/v0/models endpoint derived from the chat URL', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify({ data: [] }), { status: 200 }),
      ) as typeof globalThis.fetch;
    globalThis.fetch = fetchMock;

    await discoverLoadedModels('http://192.168.0.206:1234/v1/chat/completions', {
      Authorization: 'Bearer secret',
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = (fetchMock as ReturnType<typeof vi.fn>).mock.calls[0] as [
      string,
      RequestInit,
    ];
    expect(url).toBe('http://192.168.0.206:1234/api/v0/models');
    expect(init.method).toBe('GET');
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer secret');
  });

  it('aborts the fetch after the discovery timeout when LM Studio hangs', async () => {
    let aborted = false;
    globalThis.fetch = vi.fn((_url, init) => {
      const signal = (init as RequestInit).signal;
      return new Promise<Response>((_resolve, reject) => {
        signal?.addEventListener('abort', () => {
          aborted = true;
          const err = new Error('aborted');
          (err as Error & { name: string }).name = 'AbortError';
          reject(err);
        });
      });
    }) as typeof globalThis.fetch;

    const out = await discoverLoadedModels('http://10.0.0.1:1234/v1/chat/completions', {});
    expect(out).toEqual([]);
    expect(aborted).toBe(true);
  }, 15_000);
});

describe('registerDiscovered', () => {
  function makeModel(id: string): Model {
    return {
      id,
      provider: 'lmstudio',
      api: 'openai-completions',
      display_name: id,
      context_window: 8192,
      max_output_tokens: 8192,
      supports_thinking: false,
      supports_xhigh: false,
      supports_tools: true,
      supports_vision: false,
      supports_cache: false,
      transports: ['sse'],
    };
  }

  it('reconciles all models in one catalog write', async () => {
    const trigger = vi.fn().mockResolvedValue({ ids: ['a', 'b'], count: 2 });
    const iii = { trigger } as unknown as ISdk;

    const out = await registerDiscovered(iii, [makeModel('a'), makeModel('b')]);
    expect(out).toEqual(['a', 'b']);
    expect(trigger).toHaveBeenCalledTimes(1);
    expect(trigger).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'models::reconcile',
        payload: expect.objectContaining({
          provider: 'lmstudio',
          models: expect.arrayContaining([
            expect.objectContaining({ id: 'a' }),
            expect.objectContaining({ id: 'b' }),
          ]),
        }),
      }),
    );
  });

  it('returns [] when reconcile fails', async () => {
    const iii = {
      trigger: vi.fn().mockRejectedValue(new Error('state worker down')),
    } as unknown as ISdk;

    const out = await registerDiscovered(iii, [makeModel('a'), makeModel('b')]);
    expect(out).toEqual([]);
  });
});

describe('discoverAllDownloadedModels', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('returns every LLM/VLM regardless of state — loaded AND not-loaded', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          data: [
            { id: 'a/llm-loaded', type: 'llm', state: 'loaded' },
            { id: 'b/llm-cold', type: 'llm', state: 'not-loaded' },
            { id: 'c/embed', type: 'embeddings', state: 'loaded' },
            { id: 'd/vlm', type: 'vlm', state: 'not-loaded' },
          ],
        }),
        { status: 200 },
      ),
    ) as typeof globalThis.fetch;

    const out = await discoverAllDownloadedModels('http://localhost:1234/v1/chat/completions', {});
    expect(out.map((m) => m.id)).toEqual(['a/llm-loaded', 'b/llm-cold', 'd/vlm']);
  });

  it('returns [] when LM Studio is unreachable', async () => {
    globalThis.fetch = vi
      .fn()
      .mockRejectedValue(new Error('ECONNREFUSED')) as typeof globalThis.fetch;
    const out = await discoverAllDownloadedModels('http://localhost:1234/v1/chat/completions', {});
    expect(out).toEqual([]);
  });
});

describe('discoverLoadedIds', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('returns a Set of every currently-loaded model id', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          data: [
            { id: 'a', type: 'llm', state: 'loaded' },
            { id: 'b', type: 'llm', state: 'not-loaded' },
            { id: 'c', type: 'llm', state: 'loaded' },
          ],
        }),
        { status: 200 },
      ),
    ) as typeof globalThis.fetch;

    const out = await discoverLoadedIds('http://localhost:1234/v1/chat/completions', {});
    expect(out.has('a')).toBe(true);
    expect(out.has('b')).toBe(false);
    expect(out.has('c')).toBe(true);
    expect(out.size).toBe(2);
  });

  it('returns an empty Set when nothing is loaded', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          data: [{ id: 'a', type: 'llm', state: 'not-loaded' }],
        }),
        { status: 200 },
      ),
    ) as typeof globalThis.fetch;

    const out = await discoverLoadedIds('http://localhost:1234/v1/chat/completions', {});
    expect(out.size).toBe(0);
  });

  it('returns an empty Set when LM Studio is unreachable', async () => {
    globalThis.fetch = vi
      .fn()
      .mockRejectedValue(new Error('ECONNREFUSED')) as typeof globalThis.fetch;
    const out = await discoverLoadedIds('http://localhost:1234/v1/chat/completions', {});
    expect(out.size).toBe(0);
  });
});

describe('discoverAndRegister', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it('registers every downloaded model (loaded AND not-loaded) so the picker shows them all', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          data: [
            { id: 'qwen/qwen3-4b-2507', type: 'llm', state: 'loaded' },
            { id: 'meta/llama3.2-3b', type: 'llm', state: 'not-loaded' },
            { id: 'nomic/embed', type: 'embeddings', state: 'loaded' }, // skipped: non-llm
          ],
        }),
        { status: 200 },
      ),
    ) as typeof globalThis.fetch;

    const iii = {
      trigger: vi.fn().mockResolvedValue({ ok: true }),
    } as unknown as ISdk;

    const out = await discoverAndRegister(iii, 'http://localhost:1234/v1/chat/completions', {});
    expect(out).toEqual(['qwen/qwen3-4b-2507', 'meta/llama3.2-3b']);

    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls;
    const reconcile = calls.filter(
      (c) => (c[0] as { function_id: string }).function_id === 'models::reconcile',
    );
    expect(reconcile).toHaveLength(1);
    const payload = (reconcile[0][0] as { payload: { provider: string; models: { id: string }[] } })
      .payload;
    expect(payload.provider).toBe('lmstudio');
    expect(payload.models.map((m) => m.id)).toEqual(['qwen/qwen3-4b-2507', 'meta/llama3.2-3b']);
  });

  it('returns [] and never calls the register bus when LM Studio is unreachable', async () => {
    globalThis.fetch = vi
      .fn()
      .mockRejectedValue(new Error('ECONNREFUSED')) as typeof globalThis.fetch;
    const iii = {
      trigger: vi.fn().mockResolvedValue({ ok: true }),
    } as unknown as ISdk;

    const out = await discoverAndRegister(iii, 'http://localhost:1234/v1/chat/completions', {});
    expect(out).toEqual([]);
    expect((iii.trigger as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(0);
  });
});
