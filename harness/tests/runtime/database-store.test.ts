import { beforeEach, describe, expect, it } from 'vitest';
import { DbStore } from '../../src/runtime/database-store.js';
import { FakeDatabaseSdk } from './fake-database-sdk.js';

describe('DbStore', () => {
  let fake: FakeDatabaseSdk;

  beforeEach(() => {
    fake = new FakeDatabaseSdk();
  });

  function newStore<T>(): DbStore<T> {
    return new DbStore<T>(fake.asSdk(), { databaseName: 'harness', tableName: 'kv' });
  }

  it('init() creates the table once, idempotently', async () => {
    const s = newStore();
    await s.init();
    await s.init();
    const createCalls = fake.calls.filter(
      (c) => c.function_id === 'database::execute' && /CREATE TABLE/i.test(String((c.payload as { sql: string }).sql)),
    );
    // First init runs CREATE; second init short-circuits without a bus call.
    expect(createCalls.length).toBe(1);
    expect(fake.table('kv')).toBeDefined();
  });

  it('get returns null for missing rows', async () => {
    const s = newStore<{ key: string }>();
    await s.init();
    expect(await s.get('anthropic')).toBeNull();
  });

  it('set then get round-trips a payload', async () => {
    const s = newStore<{ key: string }>();
    await s.init();
    await s.set('anthropic', { key: 'sk-ant' });
    expect(await s.get('anthropic')).toEqual({ key: 'sk-ant' });
  });

  it('set replaces (does NOT merge) existing rows', async () => {
    type V = { a?: number; b?: number };
    const s = newStore<V>();
    await s.init();
    await s.set('p', { a: 1, b: 2 });
    await s.set('p', { a: 9 });
    expect(await s.get('p')).toEqual({ a: 9 });
  });

  it('update merges partial patches into the existing row', async () => {
    type V = { a?: number; b?: number };
    const s = newStore<V>();
    await s.init();
    await s.set('p', { a: 1, b: 2 });
    await s.update('p', { b: 99 });
    expect(await s.get('p')).toEqual({ a: 1, b: 99 });
  });

  it('clear removes the row; other rows untouched', async () => {
    const s = newStore<{ k: number }>();
    await s.init();
    await s.set('a', { k: 1 });
    await s.set('b', { k: 2 });
    await s.clear('a');
    expect(await s.get('a')).toBeNull();
    expect(await s.get('b')).toEqual({ k: 2 });
  });

  it('list returns every row sorted by provider', async () => {
    const s = newStore<{ k: number }>();
    await s.init();
    await s.set('c', { k: 3 });
    await s.set('a', { k: 1 });
    await s.set('b', { k: 2 });
    const rows = await s.list();
    expect(rows.map(([p]) => p)).toEqual(['a', 'b', 'c']);
    expect(rows.map(([, v]) => v.k)).toEqual([1, 2, 3]);
  });

  it('serializes concurrent updates so all merges land', async () => {
    type V = { x?: number; y?: number; z?: number };
    const s = newStore<V>();
    await s.init();
    await s.set('p', { x: 1 });
    await Promise.all([s.update('p', { y: 2 }), s.update('p', { z: 3 })]);
    expect(await s.get('p')).toEqual({ x: 1, y: 2, z: 3 });
  });

  it('emits the configured db name on every call', async () => {
    const s = newStore();
    await s.init();
    await s.set('a', { foo: 'bar' });
    for (const call of fake.calls) {
      const sql = (call.payload as { db: string }).db;
      expect(sql).toBe('harness');
    }
  });
});
