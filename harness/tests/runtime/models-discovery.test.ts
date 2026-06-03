import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchModelsForDiscovery, fetchModelsJson } from '../../src/runtime/models-discovery.js';

describe('fetchModelsForDiscovery', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('returns ok with parsed json on 200', async () => {
    const body = { data: [{ id: 'm1' }] };
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(body), { status: 200 }),
    ) as typeof globalThis.fetch;

    const result = await fetchModelsForDiscovery('https://api.example/v1/models', {});
    expect(result).toEqual({ kind: 'ok', json: body });
  });

  it('returns auth_error on 401 and 403', async () => {
    for (const status of [401, 403] as const) {
      globalThis.fetch = vi.fn().mockResolvedValue(
        new Response('', { status }),
      ) as typeof globalThis.fetch;
      const result = await fetchModelsForDiscovery('https://api.example/v1/models', {});
      expect(result).toEqual({ kind: 'auth_error', status });
    }
  });

  it('returns transient_error on 503', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response('', { status: 503 }),
    ) as typeof globalThis.fetch;
    const result = await fetchModelsForDiscovery('https://api.example/v1/models', {});
    expect(result).toEqual({ kind: 'transient_error', status: 503 });
  });

  it('fetchModelsJson returns null for auth_error', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response('', { status: 401 }),
    ) as typeof globalThis.fetch;
    expect(await fetchModelsJson('https://api.example/v1/models', {})).toBeNull();
  });
});
