import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, describe, expect, it } from 'vitest';
import { FileStore, InMemoryStore } from '../../src/auth-credentials/store.js';

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

describe('FileStore', () => {
  let dir: string;
  let path: string;
  const setUp = async () => {
    dir = await mkdtemp(join(tmpdir(), 'auth-creds-'));
    path = join(dir, 'creds.json');
    return new FileStore(path);
  };
  afterAll(async () => {
    if (dir) await rm(dir, { recursive: true, force: true });
  });

  it('reads as empty when file missing', async () => {
    const s = await setUp();
    expect(await s.get('anthropic')).toBeNull();
    expect(await s.list()).toEqual([]);
  });

  it('persists set across instances', async () => {
    const s = await setUp();
    await s.set('openai', { type: 'api_key', key: 'sk-op' });
    const s2 = new FileStore(path);
    expect(await s2.get('openai')).toEqual({ type: 'api_key', key: 'sk-op' });
  });

  it('serializes concurrent sets without losing entries', async () => {
    const s = await setUp();
    await Promise.all([
      s.set('a', { type: 'api_key', key: '1' }),
      s.set('b', { type: 'api_key', key: '2' }),
      s.set('c', { type: 'api_key', key: '3' }),
    ]);
    const list = await s.list();
    expect(list.length).toBe(3);
  });
});
