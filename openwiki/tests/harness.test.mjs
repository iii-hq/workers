import { test } from 'node:test';
import assert from 'node:assert/strict';
import { citationUrl, lineWindow, mapResult } from '../src/lib/harness.mjs';

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
