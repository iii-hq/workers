import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as store from '../src/lib/store.mjs';

// In-memory mock of the iii worker's state:: functions.
function mockWorker() {
  const db = new Map(); // scope -> Map(key -> value)
  const scoped = (s) => {
    if (!db.has(s)) db.set(s, new Map());
    return db.get(s);
  };
  return {
    async trigger({ function_id, payload }) {
      const { scope, key, value } = payload || {};
      if (function_id === 'state::set') {
        scoped(scope).set(key, value);
        return {};
      }
      if (function_id === 'state::get') {
        return scoped(scope).has(key) ? scoped(scope).get(key) : null;
      }
      if (function_id === 'state::list') {
        return [...scoped(scope).values()];
      }
      if (function_id === 'state::delete') {
        scoped(scope).delete(key);
        return {};
      }
      throw new Error(`unexpected ${function_id}`);
    },
  };
}

test('wiki round-trip via iii-state, listed newest first', async () => {
  store.setWorker(mockWorker());
  await store.saveWiki('w1', { id: 'w1', repo_name: 'a/b', updated_at: '2026-01-02' });
  await store.saveWiki('w2', { id: 'w2', repo_name: 'c/d', updated_at: '2026-01-03' });
  assert.equal((await store.getWiki('w1')).repo_name, 'a/b');
  const all = await store.listWikis();
  assert.equal(all.length, 2);
  assert.equal(all[0].id, 'w2');
});

test('page save / get / list / delete', async () => {
  store.setWorker(mockWorker());
  await store.savePage('w1', 'overview', '# Overview\n\nbody', { title: 'Overview', category: 'overview' });
  const p = await store.getPage('w1', 'overview');
  assert.equal(p.meta.title, 'Overview');
  assert.match(p.markdown, /# Overview/);
  const pages = await store.listPages('w1');
  assert.equal(pages.length, 1);
  assert.equal(pages[0].slug, 'overview');
  await store.deletePage('w1', 'overview');
  assert.equal(await store.getPage('w1', 'overview'), null);
});

test('status stamps updated_at; log appends and caps', async () => {
  store.setWorker(mockWorker());
  await store.updateStatus('w1', { phase: 'ready', progress: 1 });
  const s = await store.getStatus('w1');
  assert.equal(s.phase, 'ready');
  assert.ok(s.updated_at);
  await store.appendLog('w1', 'started');
  await store.appendLog('w1', 'done');
  const log = await store.getLog('w1');
  assert.equal(log.length, 2);
  assert.match(log[1], /done/);
});

test('pagesForPaths maps changed files to affected pages (source_paths + citations)', async () => {
  store.setWorker(mockWorker());
  await store.savePage('w1', 'a', '# A', { slug: 'a', title: 'A', category: 'c', source_paths: ['src/a.ts'] });
  await store.savePage('w1', 'b', '# B', {
    slug: 'b',
    title: 'B',
    category: 'c',
    citations: [{ path: 'src/b.ts', start_line: 1 }],
  });
  await store.savePage('w1', 'c', '# C', { slug: 'c', title: 'C', category: 'c', source_paths: ['src/shared.ts'] });
  await store.savePage('w1', 'd', '# D', { slug: 'd', title: 'D', category: 'c', source_paths: ['src/shared.ts'] });
  assert.deepEqual((await store.pagesForPaths('w1', ['src/a.ts'])).sort(), ['a']);
  assert.deepEqual((await store.pagesForPaths('w1', ['src/b.ts'])).sort(), ['b']);
  assert.deepEqual((await store.pagesForPaths('w1', ['src/shared.ts'])).sort(), ['c', 'd']);
  assert.deepEqual(await store.pagesForPaths('w1', ['nope.ts']), []);
  assert.deepEqual(await store.pagesForPaths('w1', []), []);
});

test('computeContentHash is order-independent and content-sensitive', async () => {
  store.setWorker(mockWorker());
  await store.savePage('w1', 'a', 'A body', { slug: 'a', title: 'A', category: 'c' });
  await store.savePage('w1', 'b', 'B body', { slug: 'b', title: 'B', category: 'c' });
  const h1 = await store.computeContentHash('w1');

  store.setWorker(mockWorker()); // fresh store, pages written in the other order
  await store.savePage('w1', 'b', 'B body', { slug: 'b', title: 'B', category: 'c' });
  await store.savePage('w1', 'a', 'A body', { slug: 'a', title: 'A', category: 'c' });
  assert.equal(await store.computeContentHash('w1'), h1);

  await store.savePage('w1', 'a', 'A body CHANGED', { slug: 'a', title: 'A', category: 'c' });
  assert.notEqual(await store.computeContentHash('w1'), h1);
});

test('listPages reads the side-index, never enumerates page bodies', async () => {
  const base = mockWorker();
  let pageListCalls = 0;
  const spy = {
    async trigger(req) {
      if (req.function_id === 'state::list' && String(req.payload?.scope || '').startsWith('openwiki:pages:'))
        pageListCalls++;
      return base.trigger(req);
    },
  };
  store.setWorker(spy);
  await store.savePage('w1', 'a', '# A body', { slug: 'a', title: 'A', category: 'c' });
  await store.savePage('w1', 'b', '# B body', { slug: 'b', title: 'B', category: 'c' });
  const pages = await store.listPages('w1');
  assert.equal(pages.length, 2);
  assert.equal(pageListCalls, 0);
});
