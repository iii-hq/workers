import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { askDeep } from '../src/lib/ask.mjs';
import {
  citationUrl,
  generatePageViaHarness,
  lineWindow,
  mapResult,
  newRootSession,
  planViaHarness,
  runOrchestrator,
} from '../src/lib/harness.mjs';

// Records every harness::send payload and finishes the turn on the first status poll.
function fakeWorker(result) {
  const sends = [];
  return {
    sends,
    async call({ function_id, payload }) {
      if (function_id === 'router::models::list') return { models: [{ id: 'm', provider: 'p', supports_tools: true }] };
      if (function_id === 'harness::send') {
        sends.push(payload);
        return { session_id: payload.session_id || 's_new' };
      }
      if (function_id === 'harness::status') return { status: 'completed', result };
      throw new Error(`unexpected call ${function_id}`);
    },
  };
}

test('citationUrl builds a GitHub blob permalink at the pinned commit', () => {
  assert.equal(
    citationUrl('https://github.com/owner/repo', 'abc', 'src/a.ts', 10, 24),
    'https://github.com/owner/repo/blob/abc/src/a.ts#L10-L24',
  );
  assert.equal(
    citationUrl('git@github.com:owner/repo.git', 'abc', 'x.ts', 5),
    'https://github.com/owner/repo/blob/abc/x.ts#L5',
  );
  assert.equal(citationUrl('https://gitlab.com/o/r', 'abc', 'x.ts', 5), null);
  assert.equal(citationUrl('https://github.com/o/r', null, 'x.ts', 5), null);
});

test('lineWindow slices inclusive 1-indexed ranges', () => {
  const c = 'l1\nl2\nl3\nl4';
  assert.deepEqual(lineWindow(c, 2, 3), { text: 'l2\nl3', from: 2, to: 3, total_lines: 4, truncated: true });
  assert.deepEqual(lineWindow(c), { text: c, from: 1, to: 4, total_lines: 4, truncated: false });
});

test('mapResult attaches citation urls, source paths, and defaults', () => {
  const out = mapResult(
    { title: 'T', markdown: '# T\nbody', citations: [{ path: 'a.ts', start_line: 1, end_line: 2 }] },
    {
      outlineItem: { slug: 's', category: 'c', source_paths: ['a.ts'] },
      repoUrl: 'https://github.com/o/r',
      commit: 'sha',
    },
  );
  assert.equal(out.frontmatter.generator, 'harness');
  assert.equal(out.frontmatter.citations[0].url, 'https://github.com/o/r/blob/sha/a.ts#L1-L2');
  assert.equal(out.frontmatter.confidence, 'medium');
  assert.equal(out.frontmatter.status, 'current');
  assert.ok(out.frontmatter.source_paths.includes('a.ts'));
});

test('mapResult throws on empty markdown', () => {
  assert.throws(() =>
    mapResult({ title: 'T', markdown: '   ' }, { outlineItem: { slug: 's' }, repoUrl: '', commit: '' }),
  );
});

test('newRootSession gives every run its own id under the wiki prefix', () => {
  const a = newRootSession('My Repo', 'abcdef12-3456');
  const b = newRootSession('My Repo', 'abcdef12-3456');
  assert.notEqual(a, b);
  assert.match(a, /^openwiki:my-repo:abcdef:[0-9a-f]{8}$/);
  assert.match(b, /^openwiki:my-repo:abcdef:[0-9a-f]{8}$/);
});

test('the lead turn runs in an automation session on the given root', async () => {
  const w = fakeWorker({ navigation: [] });
  await runOrchestrator(w, { rootSessionId: 'root-1', wikiId: 'w', repoName: 'r', repoUrl: 'u', model: 'm' });
  assert.equal(w.sends.length, 1);
  assert.equal(w.sends[0].session_id, 'root-1');
  assert.deepEqual(w.sends[0].session, { title: 'openwiki: r', kind: 'automation' });
  assert.ok(w.sends[0].message.includes('"root-1/<slug>"'));
});

test('the plan turn runs in an automation session on the given root', async () => {
  const w = fakeWorker({ pages: [{ slug: 'a', title: 'A' }], navigation: [] });
  const plan = await planViaHarness(w, {
    rootSessionId: 'root-1',
    wikiId: 'w',
    repoName: 'r',
    repoUrl: 'u',
    model: 'm',
  });
  assert.equal(w.sends[0].session_id, 'root-1');
  assert.equal(w.sends[0].session.kind, 'automation');
  assert.equal(plan.sessionId, 'root-1');
});

test('the page writer creates an automation session with and without a child session id', async () => {
  const opts = {
    wikiId: 'w',
    outlineItem: { slug: 's', title: 'T', category: 'c' },
    repoName: 'r',
    repoUrl: 'u',
    commit: 'c',
    model: 'm',
    maxAttempts: 1,
  };
  const nested = fakeWorker({ markdown: '# T\nbody' });
  await generatePageViaHarness(nested, { ...opts, parentSessionId: 'root-1' });
  assert.equal(nested.sends[0].session_id, 'root-1/s');
  assert.deepEqual(nested.sends[0].session, {
    title: 'T',
    metadata: { parent_session_id: 'root-1', depth: 1 },
    kind: 'automation',
  });
  // Prevents: a router-fallback plan or an incremental refresh (no parent) starting a `user` session.
  const top = fakeWorker({ markdown: '# T\nbody' });
  await generatePageViaHarness(top, { ...opts, parentSessionId: undefined });
  assert.ok(!('session_id' in top.sends[0]));
  assert.deepEqual(top.sends[0].session, { kind: 'automation' });
});

test('deep ask creates an automation session', async () => {
  const w = fakeWorker('the answer');
  assert.equal(await askDeep(w, { id: 'w', q: 'how?', model: 'm', blocks: [] }), 'the answer');
  assert.ok(!('session_id' in w.sends[0]));
  assert.deepEqual(w.sends[0].session, { kind: 'automation' });
});

test('harness::send has one call site and a run mints its root once', () => {
  const lib = new URL('../src/lib/', import.meta.url);
  const INDEX = fs.readFileSync(new URL('../src/index.mjs', import.meta.url), 'utf8');
  const all = [
    INDEX,
    ...fs
      .readdirSync(lib)
      .filter((f) => f.endsWith('.mjs'))
      .map((f) => fs.readFileSync(new URL(f, lib), 'utf8')),
  ].join('\n');
  // Prevents: a new send or spawn site that bypasses the automation kind.
  assert.equal((all.match(/function_id:\s*'harness::(send|spawn)'/g) || []).length, 1);
  assert.equal((INDEX.match(/newRootSession\(/g) || []).length, 1);
  assert.match(INDEX, /turnbus\.register\(orchRoot,/);
  assert.match(INDEX, /runOrchestrator\(worker, \{\s*rootSessionId: orchRoot,/);
  assert.match(INDEX, /planViaHarness\(worker, \{\s*rootSessionId: orchRoot,/);
});
