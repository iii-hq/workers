import { beforeEach, describe, expect, it } from 'vitest';
import { loadProviderConfigConfig } from '../../src/provider-config/config.js';
import {
  DbOverridesStore,
  InMemoryStore,
  PROVIDER_CONFIG_TABLE,
} from '../../src/provider-config/store.js';
import { createDbStore } from '../../src/runtime/database-store.js';
import { FakeDatabaseSdk } from '../runtime/fake-database-sdk.js';

describe('InMemoryStore (provider-config)', () => {
  it('round-trips set/get/clear/list', async () => {
    const s = new InMemoryStore();
    await s.set('anthropic', { default_api_url: 'https://x/v1', default_max_tokens: 100 });
    expect(await s.get('anthropic')).toEqual({
      default_api_url: 'https://x/v1',
      default_max_tokens: 100,
    });
    await s.set('openai', { default_max_tokens: 4096 });
    expect((await s.list()).length).toBe(2);
    await s.clear('anthropic');
    expect(await s.get('anthropic')).toBeNull();
    expect((await s.list()).length).toBe(1);
  });

  it('returns null for an unknown provider', async () => {
    const s = new InMemoryStore();
    expect(await s.get('unknown')).toBeNull();
  });
});

describe('DbOverridesStore', () => {
  let fake: FakeDatabaseSdk;
  let store: DbOverridesStore;

  beforeEach(async () => {
    fake = new FakeDatabaseSdk();
    store = new DbOverridesStore(fake.asSdk(), { databaseName: 'harness' });
    await store.init();
  });

  it('writes into the provider_config table', async () => {
    await store.set('openai', { default_max_tokens: 4096 });
    expect(fake.table(PROVIDER_CONFIG_TABLE)?.size).toBe(1);
  });

  it('reads as empty before any writes', async () => {
    expect(await store.get('anthropic')).toBeNull();
    expect(await store.list()).toEqual([]);
  });

  it('persists set across new wrapper instances on the same fake', async () => {
    await store.set('openai', {
      default_api_url: 'https://example.com/v1',
      default_max_tokens: 4096,
    });
    const store2 = new DbOverridesStore(fake.asSdk(), { databaseName: 'harness' });
    await store2.init();
    expect(await store2.get('openai')).toEqual({
      default_api_url: 'https://example.com/v1',
      default_max_tokens: 4096,
    });
  });

  it('merges partial updates into an existing entry (set is merge, not replace)', async () => {
    await store.set('anthropic', {
      default_api_url: 'https://api.anthropic.com/v1/messages',
      default_max_tokens: 8192,
    });
    await store.set('anthropic', { default_max_tokens: 4096 });
    expect(await store.get('anthropic')).toEqual({
      default_api_url: 'https://api.anthropic.com/v1/messages',
      default_max_tokens: 4096,
    });
  });

  it('serializes concurrent sets without losing entries', async () => {
    await Promise.all([
      store.set('a', { default_max_tokens: 1 }),
      store.set('b', { default_max_tokens: 2 }),
      store.set('c', { default_max_tokens: 3 }),
    ]);
    const list = await store.list();
    expect(list.length).toBe(3);
    const byProvider = Object.fromEntries(list);
    expect(byProvider.a?.default_max_tokens).toBe(1);
    expect(byProvider.b?.default_max_tokens).toBe(2);
    expect(byProvider.c?.default_max_tokens).toBe(3);
  });

  it('clear leaves other entries intact', async () => {
    await store.set('a', { default_max_tokens: 1 });
    await store.set('b', { default_max_tokens: 2 });
    await store.clear('a');
    expect(await store.get('a')).toBeNull();
    expect(await store.get('b')).toEqual({ default_max_tokens: 2 });
  });

  it('resolves database_name from shared storage when worker section has no override', async () => {
    const cfg = { storage: { database_name: 'shared-pool' } };
    expect(loadProviderConfigConfig(cfg).database_name).toBe('shared-pool');

    const sharedFake = new FakeDatabaseSdk();
    const sharedStore = createDbStore<{ default_max_tokens?: number }>(sharedFake.asSdk(), cfg, {
      workerSection: 'provider_config',
      tableName: PROVIDER_CONFIG_TABLE,
    });
    await sharedStore.init();
    await sharedStore.set('openai', { default_max_tokens: 4096 });
    for (const call of sharedFake.calls) {
      expect((call.payload as { db: string }).db).toBe('shared-pool');
    }
  });
});
