import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  _resetModelsDevForTests,
  getModelsDevIndex,
  lookupModelsDev,
  MODELSDEV_URL,
} from '../../src/runtime/modelsdev.js';

const API_JSON = {
  anthropic: {
    id: 'anthropic',
    models: {
      'claude-sonnet-4-6': {
        id: 'claude-sonnet-4-6',
        reasoning: true,
        tool_call: true,
        limit: { context: 200_000, output: 64_000 },
      },
      'claude-sonnet-4': {
        id: 'claude-sonnet-4',
        reasoning: true,
        tool_call: true,
        limit: { context: 200_000, output: 64_000 },
      },
    },
  },
  openai: {
    id: 'openai',
    models: {
      'gpt-5-pro': {
        id: 'gpt-5-pro',
        reasoning: true,
        tool_call: true,
        limit: { context: 400_000, output: 272_000 },
      },
    },
  },
  moonshotai: {
    id: 'moonshotai',
    models: {
      'kimi-k2.6': { id: 'kimi-k2.6', reasoning: true, limit: { context: 256_000, output: 8_192 } },
    },
  },
};

afterEach(() => {
  vi.unstubAllGlobals();
  _resetModelsDevForTests();
});

function stubFetch(impl: () => Promise<Response>): ReturnType<typeof vi.fn> {
  const mock = vi.fn().mockImplementation(impl);
  vi.stubGlobal('fetch', mock);
  return mock;
}

describe('getModelsDevIndex', () => {
  it('parses providers and model limits from api.json', async () => {
    stubFetch(async () => new Response(JSON.stringify(API_JSON), { status: 200 }));
    const index = await getModelsDevIndex();
    const sonnet = lookupModelsDev(index, 'anthropic', 'claude-sonnet-4-6');
    expect(sonnet?.limit).toEqual({ context: 200_000, input: undefined, output: 64_000 });
    expect(sonnet?.reasoning).toBe(true);
  });

  it('returns an empty index on non-200', async () => {
    stubFetch(async () => new Response('', { status: 503 }));
    const index = await getModelsDevIndex();
    expect(index.size).toBe(0);
  });

  it('returns an empty index when fetch throws', async () => {
    stubFetch(async () => {
      throw new Error('network down');
    });
    const index = await getModelsDevIndex();
    expect(index.size).toBe(0);
  });

  it('returns an empty index on malformed JSON', async () => {
    stubFetch(async () => new Response('not-json', { status: 200 }));
    const index = await getModelsDevIndex();
    expect(index.size).toBe(0);
  });

  it('caches the result: two calls, one fetch', async () => {
    const mock = stubFetch(async () => new Response(JSON.stringify(API_JSON), { status: 200 }));
    await getModelsDevIndex();
    await getModelsDevIndex();
    expect(mock).toHaveBeenCalledTimes(1);
    expect(mock).toHaveBeenCalledWith(MODELSDEV_URL, expect.anything());
  });

  it('dedupes concurrent fetches in flight', async () => {
    const mock = stubFetch(async () => new Response(JSON.stringify(API_JSON), { status: 200 }));
    await Promise.all([getModelsDevIndex(), getModelsDevIndex(), getModelsDevIndex()]);
    expect(mock).toHaveBeenCalledTimes(1);
  });

  it('caches a failed fetch: does not re-hit models.dev within the failure TTL', async () => {
    const mock = stubFetch(async () => new Response('', { status: 503 }));
    const first = await getModelsDevIndex();
    const second = await getModelsDevIndex();
    expect(first.size).toBe(0);
    expect(second.size).toBe(0);
    expect(mock).toHaveBeenCalledTimes(1);
  });

  it('rejects out-of-range limit values (sanity bounds)', async () => {
    const poisoned = {
      anthropic: {
        id: 'anthropic',
        models: {
          'claude-x': { id: 'claude-x', limit: { context: 200_000, output: 1 } },
          'claude-y': { id: 'claude-y', limit: { context: 200_000, output: 99_000_000 } },
        },
      },
    };
    stubFetch(async () => new Response(JSON.stringify(poisoned), { status: 200 }));
    const idx = await getModelsDevIndex();
    expect(lookupModelsDev(idx, 'anthropic', 'claude-x')?.limit?.output).toBeUndefined();
    expect(lookupModelsDev(idx, 'anthropic', 'claude-y')?.limit?.output).toBeUndefined();
    // context within range still accepted
    expect(lookupModelsDev(idx, 'anthropic', 'claude-x')?.limit?.context).toBe(200_000);
  });
});

describe('lookupModelsDev', () => {
  async function index() {
    stubFetch(async () => new Response(JSON.stringify(API_JSON), { status: 200 }));
    return getModelsDevIndex();
  }

  it('matches exact model ids', async () => {
    const idx = await index();
    expect(lookupModelsDev(idx, 'openai', 'gpt-5-pro')?.limit?.output).toBe(272_000);
  });

  it('matches date-suffixed ids against the undated catalog id', async () => {
    const idx = await index();
    expect(lookupModelsDev(idx, 'anthropic', 'claude-sonnet-4-20250514')?.id).toBe(
      'claude-sonnet-4',
    );
  });

  it('maps the kimi provider id to moonshotai', async () => {
    const idx = await index();
    expect(lookupModelsDev(idx, 'kimi', 'kimi-k2.6')?.limit?.context).toBe(256_000);
  });

  it.each(['lmstudio', 'llamacpp', 'unknown'])('returns undefined for provider %s', async (p) => {
    const idx = await index();
    expect(lookupModelsDev(idx, p, 'anything')).toBeUndefined();
  });

  it('returns undefined for unknown model ids', async () => {
    const idx = await index();
    expect(lookupModelsDev(idx, 'anthropic', 'claude-nonexistent-9')).toBeUndefined();
  });

  it('matches an undated lookup id against a date-suffixed catalog id (loop scan)', async () => {
    const json = {
      anthropic: {
        id: 'anthropic',
        models: {
          'claude-opus-4-20251101': {
            id: 'claude-opus-4-20251101',
            limit: { context: 200_000, output: 32_000 },
          },
        },
      },
    };
    stubFetch(async () => new Response(JSON.stringify(json), { status: 200 }));
    const idx = await getModelsDevIndex();
    expect(lookupModelsDev(idx, 'anthropic', 'claude-opus-4')?.id).toBe('claude-opus-4-20251101');
  });
});
