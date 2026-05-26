import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  discoverAndRegister,
  discoverLoadedModel,
  modelsUrl,
  propsUrl,
  registerDiscovered,
} from '../../src/provider-llamacpp/discover.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { Model } from '../../src/models-catalog/types.js';

const originalFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

describe('modelsUrl', () => {
  it('derives /v1/models from the default chat URL', () => {
    expect(modelsUrl('http://localhost:8080/v1/chat/completions')).toBe(
      'http://localhost:8080/v1/models',
    );
  });

  it('strips a trailing slash before appending', () => {
    expect(modelsUrl('http://localhost:8080/v1/chat/completions/')).toBe(
      'http://localhost:8080/v1/models',
    );
  });

  it('falls through for non-canonical paths (appends `/models`)', () => {
    expect(modelsUrl('http://localhost:8080/custom/path')).toBe(
      'http://localhost:8080/custom/path/models',
    );
  });
});

describe('propsUrl', () => {
  it('derives /props at the server root from the default chat URL', () => {
    expect(propsUrl('http://localhost:8080/v1/chat/completions')).toBe(
      'http://localhost:8080/props',
    );
  });

  it('strips a trailing slash on the chat URL before deriving /props', () => {
    expect(propsUrl('http://localhost:8080/v1/chat/completions/')).toBe(
      'http://localhost:8080/props',
    );
  });

  it('strips a bare /v1 suffix', () => {
    expect(propsUrl('http://localhost:8080/v1')).toBe('http://localhost:8080/props');
  });

  it('appends /props to a custom-path proxy without /v1 (mirrors modelsUrl)', () => {
    expect(propsUrl('http://localhost:8080/custom/path')).toBe(
      'http://localhost:8080/custom/path/props',
    );
  });
});

describe('discoverLoadedModel', () => {
  it('returns one Model for the single loaded llama-server entry', async () => {
    globalThis.fetch = vi.fn(
      async () =>
        new Response(JSON.stringify({ data: [{ id: 'Meta-Llama-3.1-8B-Instruct' }] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    ) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out).toHaveLength(1);
    expect(out[0]?.id).toBe('Meta-Llama-3.1-8B-Instruct');
    expect(out[0]?.provider).toBe('llamacpp');
    expect(out[0]?.supports_tools).toBe(true);
  });

  it('returns empty on a non-2xx response (best-effort)', async () => {
    globalThis.fetch = vi.fn(
      async () => new Response('boom', { status: 502 }),
    ) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out).toEqual([]);
  });

  it('returns empty when the response is malformed JSON', async () => {
    globalThis.fetch = vi.fn(
      async () => new Response('not-json', { status: 200 }),
    ) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out).toEqual([]);
  });

  it('returns empty when fetch throws (network error)', async () => {
    globalThis.fetch = vi.fn(async () => {
      throw new Error('ECONNREFUSED');
    }) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out).toEqual([]);
  });

  it('skips entries without a valid id', async () => {
    globalThis.fetch = vi.fn(
      async () =>
        new Response(JSON.stringify({ data: [{ id: '' }, { foo: 'bar' }, { id: 'valid' }] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    ) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out.map((m) => m.id)).toEqual(['valid']);
  });

  it("populates context_window from /props.n_ctx when available", async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.endsWith('/props')) {
        return new Response(JSON.stringify({ n_ctx: 262_144 }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({ data: [{ id: 'Qwen3-35B-A3B' }] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out).toHaveLength(1);
    expect(out[0]?.context_window).toBe(262_144);
  });

  it('falls back to the default context_window when /props is unavailable', async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.endsWith('/props')) {
        return new Response('boom', { status: 502 });
      }
      return new Response(JSON.stringify({ data: [{ id: 'Qwen3-35B-A3B' }] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out).toHaveLength(1);
    expect(out[0]?.context_window).toBe(32_768);
  });

  it('reads context_window from /props.default_generation_settings.n_ctx (older llama-server)', async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.endsWith('/props')) {
        return new Response(
          JSON.stringify({ default_generation_settings: { n_ctx: 131_072 } }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response(JSON.stringify({ data: [{ id: 'Qwen3-35B-A3B' }] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out[0]?.context_window).toBe(131_072);
  });

  it('prefers top-level /props.n_ctx over the nested default_generation_settings value', async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.endsWith('/props')) {
        return new Response(
          JSON.stringify({
            n_ctx: 262_144,
            default_generation_settings: { n_ctx: 4096 },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response(JSON.stringify({ data: [{ id: 'Qwen3-35B-A3B' }] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out[0]?.context_window).toBe(262_144);
  });

  it('falls back to the default context_window when /props omits or invalidates n_ctx', async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.endsWith('/props')) {
        return new Response(JSON.stringify({ n_ctx: -1 }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({ data: [{ id: 'Qwen3-35B-A3B' }] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }) as typeof globalThis.fetch;
    const out = await discoverLoadedModel('http://localhost:8080/v1/chat/completions', {});
    expect(out[0]?.context_window).toBe(32_768);
  });
});

describe('registerDiscovered', () => {
  it('fans out registers in parallel and collects fulfilled ids', async () => {
    const trigger = vi.fn().mockResolvedValue(undefined);
    const sdk = { trigger, registerFunction: vi.fn() } as unknown as ISdk;
    const models: Model[] = [
      {
        id: 'a',
        provider: 'llamacpp',
        api: 'openai-completions',
        display_name: 'a',
        context_window: 1,
      },
      {
        id: 'b',
        provider: 'llamacpp',
        api: 'openai-completions',
        display_name: 'b',
        context_window: 1,
      },
    ];
    const out = await registerDiscovered(sdk, models);
    expect(out.sort()).toEqual(['a', 'b']);
    expect(trigger).toHaveBeenCalledTimes(2);
  });

  it('logs and skips failing registers but keeps the successes', async () => {
    const trigger = vi
      .fn()
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error('boom'));
    const sdk = { trigger, registerFunction: vi.fn() } as unknown as ISdk;
    const models: Model[] = [
      {
        id: 'a',
        provider: 'llamacpp',
        api: 'openai-completions',
        display_name: 'a',
        context_window: 1,
      },
      {
        id: 'b',
        provider: 'llamacpp',
        api: 'openai-completions',
        display_name: 'b',
        context_window: 1,
      },
    ];
    const out = await registerDiscovered(sdk, models);
    expect(out).toEqual(['a']);
  });
});

describe('discoverAndRegister', () => {
  it('returns an empty list when no models are reported', async () => {
    globalThis.fetch = vi.fn(
      async () =>
        new Response(JSON.stringify({ data: [] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    ) as typeof globalThis.fetch;
    const trigger = vi.fn();
    const sdk = { trigger, registerFunction: vi.fn() } as unknown as ISdk;
    const out = await discoverAndRegister(sdk, 'http://localhost:8080/v1/chat/completions', {});
    expect(out).toEqual([]);
    expect(trigger).not.toHaveBeenCalled();
  });

  it('end-to-end: discovers and registers the loaded model id', async () => {
    globalThis.fetch = vi.fn(
      async () =>
        new Response(JSON.stringify({ data: [{ id: 'Meta-Llama-3.1-8B' }] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    ) as typeof globalThis.fetch;
    const trigger = vi.fn().mockResolvedValue(undefined);
    const sdk = { trigger, registerFunction: vi.fn() } as unknown as ISdk;
    const out = await discoverAndRegister(sdk, 'http://localhost:8080/v1/chat/completions', {});
    expect(out).toEqual(['Meta-Llama-3.1-8B']);
    expect(trigger).toHaveBeenCalledWith(
      expect.objectContaining({ function_id: 'models::register' }),
    );
  });
});
