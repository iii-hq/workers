import { describe, expect, it, vi } from 'vitest';
import { register as registerClear } from '../../src/provider-config/handlers/clear.js';
import { register as registerGet } from '../../src/provider-config/handlers/get.js';
import { register as registerList } from '../../src/provider-config/handlers/list.js';
import { register as registerSet } from '../../src/provider-config/handlers/set.js';
import { InMemoryStore } from '../../src/provider-config/store.js';
import type { ISdk } from '../../src/runtime/iii.js';

type Handler = (payload: unknown) => Promise<unknown>;

function makeSdk(): { iii: ISdk; handlers: Map<string, Handler> } {
  const handlers = new Map<string, Handler>();
  const iii = {
    registerFunction: vi.fn((fnId: string, handler: Handler) => {
      handlers.set(fnId, handler);
    }),
    trigger: vi.fn(),
  } as unknown as ISdk;
  return { iii, handlers };
}

function setupAll() {
  const store = new InMemoryStore();
  const { iii, handlers } = makeSdk();
  registerGet(iii, store);
  registerSet(iii, store);
  registerClear(iii, store);
  registerList(iii, store);
  return {
    store,
    get: (payload: unknown) => handlers.get('provider_config::get')!(payload),
    set: (payload: unknown) => handlers.get('provider_config::set')!(payload),
    clear: (payload: unknown) => handlers.get('provider_config::clear')!(payload),
    list: (payload?: unknown) => handlers.get('provider_config::list')!(payload),
  };
}

describe('provider_config::get', () => {
  it('returns stored overrides', async () => {
    const { store, get } = setupAll();
    await store.set('anthropic', {
      default_api_url: 'https://api.anthropic.com/v1/messages',
      default_max_tokens: 4096,
    });
    expect(await get({ provider: 'anthropic' })).toEqual({
      default_api_url: 'https://api.anthropic.com/v1/messages',
      default_max_tokens: 4096,
    });
  });

  it('returns {} for unknown provider', async () => {
    const { get } = setupAll();
    expect(await get({ provider: 'nope' })).toEqual({});
  });

  it('throws when provider is missing', async () => {
    const { get } = setupAll();
    await expect(get({})).rejects.toThrow();
  });
});

describe('provider_config::set', () => {
  it('persists default_api_url alone', async () => {
    const { store, set } = setupAll();
    await set({ provider: 'anthropic', default_api_url: 'https://x/v1' });
    expect(await store.get('anthropic')).toEqual({ default_api_url: 'https://x/v1' });
  });

  it('persists default_max_tokens alone', async () => {
    const { store, set } = setupAll();
    await set({ provider: 'openai', default_max_tokens: 4096 });
    expect(await store.get('openai')).toEqual({ default_max_tokens: 4096 });
  });

  it('merges partial updates with existing entry', async () => {
    const { store, set } = setupAll();
    await set({ provider: 'anthropic', default_api_url: 'https://x/v1' });
    await set({ provider: 'anthropic', default_max_tokens: 4096 });
    expect(await store.get('anthropic')).toEqual({
      default_api_url: 'https://x/v1',
      default_max_tokens: 4096,
    });
  });

  it('returns { ok: true }', async () => {
    const { set } = setupAll();
    expect(await set({ provider: 'anthropic', default_max_tokens: 4096 })).toEqual({ ok: true });
  });

  it('rejects when neither field is provided', async () => {
    const { set } = setupAll();
    await expect(set({ provider: 'anthropic' })).rejects.toThrow(
      /at least one of default_api_url or default_max_tokens/,
    );
  });

  it('rejects invalid URL: not a URL', async () => {
    const { set } = setupAll();
    await expect(
      set({ provider: 'anthropic', default_api_url: 'not-a-url' }),
    ).rejects.toThrow(/not a valid URL/);
  });

  it('rejects invalid URL: wrong scheme', async () => {
    const { set } = setupAll();
    await expect(
      set({ provider: 'anthropic', default_api_url: 'ftp://example.com' }),
    ).rejects.toThrow(/http or https/);
  });

  it('rejects empty URL string', async () => {
    const { set } = setupAll();
    await expect(
      set({ provider: 'anthropic', default_api_url: '' }),
    ).rejects.toThrow(/non-empty string/);
  });

  it('rejects non-integer max_tokens', async () => {
    const { set } = setupAll();
    await expect(
      set({ provider: 'anthropic', default_max_tokens: 1.5 }),
    ).rejects.toThrow(/integer/);
  });

  it('rejects negative max_tokens', async () => {
    const { set } = setupAll();
    await expect(
      set({ provider: 'anthropic', default_max_tokens: -1 }),
    ).rejects.toThrow(/positive integer/);
  });

  it('rejects zero max_tokens', async () => {
    const { set } = setupAll();
    await expect(
      set({ provider: 'anthropic', default_max_tokens: 0 }),
    ).rejects.toThrow(/positive integer/);
  });

  it('rejects max_tokens exceeding the limit', async () => {
    const { set } = setupAll();
    await expect(
      set({ provider: 'anthropic', default_max_tokens: 1_048_577 }),
    ).rejects.toThrow(/<=/);
  });

  it('accepts max_tokens exactly at the limit', async () => {
    const { store, set } = setupAll();
    await set({ provider: 'anthropic', default_max_tokens: 1_048_576 });
    expect(await store.get('anthropic')).toEqual({ default_max_tokens: 1_048_576 });
  });

  it('throws when provider is missing', async () => {
    const { set } = setupAll();
    await expect(set({ default_max_tokens: 1 })).rejects.toThrow();
  });
});

describe('provider_config::clear', () => {
  it('removes the entry and leaves others intact', async () => {
    const { store, clear } = setupAll();
    await store.set('a', { default_max_tokens: 1 });
    await store.set('b', { default_max_tokens: 2 });
    await clear({ provider: 'a' });
    expect(await store.get('a')).toBeNull();
    expect(await store.get('b')).toEqual({ default_max_tokens: 2 });
  });

  it('is idempotent for unknown providers', async () => {
    const { clear } = setupAll();
    await expect(clear({ provider: 'nope' })).resolves.toEqual({ ok: true });
  });

  it('throws when provider is missing', async () => {
    const { clear } = setupAll();
    await expect(clear({})).rejects.toThrow();
  });
});

describe('provider_config::list', () => {
  it('returns all entries as { provider, overrides } pairs', async () => {
    const { store, list } = setupAll();
    await store.set('a', { default_max_tokens: 1 });
    await store.set('b', { default_api_url: 'https://b/v1' });
    const result = (await list()) as { entries: Array<{ provider: string; overrides: unknown }> };
    expect(result.entries.length).toBe(2);
    const byProvider = Object.fromEntries(result.entries.map((e) => [e.provider, e.overrides]));
    expect(byProvider.a).toEqual({ default_max_tokens: 1 });
    expect(byProvider.b).toEqual({ default_api_url: 'https://b/v1' });
  });

  it('returns an empty list when nothing is stored', async () => {
    const { list } = setupAll();
    expect(await list()).toEqual({ entries: [] });
  });
});
