import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseNameStatus, repoName } from '../src/lib/git.mjs';

test('repoName extracts owner/repo from common url shapes', () => {
  assert.equal(repoName('https://github.com/owner/repo.git'), 'owner/repo');
  assert.equal(repoName('git@github.com:owner/repo.git'), 'owner/repo');
  assert.equal(repoName('https://github.com/owner/repo/'), 'owner/repo');
  assert.equal(repoName('https://example.com/deep/path/owner/repo'), 'owner/repo');
});

test('parseNameStatus handles added/modified/deleted/renamed and blanks', () => {
  const out = parseNameStatus('M\tsrc/a.ts\nA\tsrc/b.ts\nD\tsrc/c.ts\nR100\told.ts\tnew.ts\n\n');
  assert.deepEqual(out, [
    { status: 'M', path: 'src/a.ts' },
    { status: 'A', path: 'src/b.ts' },
    { status: 'D', path: 'src/c.ts' },
    { status: 'R', path: 'new.ts' },
  ]);
  assert.deepEqual(parseNameStatus(''), []);
});
