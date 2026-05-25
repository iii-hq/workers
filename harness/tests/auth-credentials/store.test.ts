import { beforeEach, describe, expect, it } from 'vitest';
import { loadAuthCredentialsConfig } from '../../src/auth-credentials/config.js';
import { CREDENTIALS_TABLE, DbCredentialStore, InMemoryStore } from '../../src/auth-credentials/store.js';
import { createDbStore } from '../../src/runtime/database-store.js';
import { FakeDatabaseSdk } from '../runtime/fake-database-sdk.js';

describe('InMemoryStore', () => {
  it('round-trips set/get/clear/list', async () => {
    const s = new InMemoryStore();
    await s.set('anthropic', { type: 'api_key', key: 'sk-ant-xxx' });
    expect(await s.get('anthropic')).toEqual({ type: 'api_key', key: 'sk-ant-xxx' });
    await s.set('openai', { type: 'api_key', key: 'sk-op' });
    expect((await s.list()).length).toBe(2);
    await s.clear('anthropic');
    expect(await s.get('anthropic')).toBeNull();
  });
});

describe('DbCredentialStore', () => {
  let fake: FakeDatabaseSdk;
  let store: DbCredentialStore;

  beforeEach(async () => {
    fake = new FakeDatabaseSdk();
    store = new DbCredentialStore(fake.asSdk(), { databaseName: 'harness' });
    await store.init();
  });

  it('writes into the auth_credentials table', async () => {
    await store.set('openai', { type: 'api_key', key: 'sk-op' });
    expect(fake.table(CREDENTIALS_TABLE)?.size).toBe(1);
  });

  it('persists set across new wrapper instances on the same fake', async () => {
    await store.set('openai', { type: 'api_key', key: 'sk-op' });
    const store2 = new DbCredentialStore(fake.asSdk(), { databaseName: 'harness' });
    await store2.init();
    expect(await store2.get('openai')).toEqual({ type: 'api_key', key: 'sk-op' });
  });

  it('replace semantics on repeated set (no merge)', async () => {
    await store.set('anthropic', { type: 'api_key', key: 'first' });
    await store.set('anthropic', { type: 'api_key', key: 'second' });
    expect(await store.get('anthropic')).toEqual({ type: 'api_key', key: 'second' });
  });

  it('concurrent sets are independent UPSERTs (no lost rows)', async () => {
    await Promise.all([
      store.set('a', { type: 'api_key', key: '1' }),
      store.set('b', { type: 'api_key', key: '2' }),
      store.set('c', { type: 'api_key', key: '3' }),
    ]);
    expect((await store.list()).length).toBe(3);
  });

  it('clear removes only the target row', async () => {
    await store.set('a', { type: 'api_key', key: '1' });
    await store.set('b', { type: 'api_key', key: '2' });
    await store.clear('a');
    expect(await store.get('a')).toBeNull();
    expect(await store.get('b')).toEqual({ type: 'api_key', key: '2' });
  });

  it('resolves database_name from shared storage when worker section has no override', async () => {
    const cfg = { storage: { database_name: 'shared-pool' } };
    expect(loadAuthCredentialsConfig(cfg).database_name).toBe('shared-pool');

    const sharedFake = new FakeDatabaseSdk();
    const sharedStore = createDbStore<{ type: 'api_key'; key: string }>(sharedFake.asSdk(), cfg, {
      workerSection: 'auth_credentials',
      tableName: CREDENTIALS_TABLE,
    });
    await sharedStore.init();
    await sharedStore.set('openai', { type: 'api_key', key: 'sk-op' });
    for (const call of sharedFake.calls) {
      expect((call.payload as { db: string }).db).toBe('shared-pool');
    }
  });
});
