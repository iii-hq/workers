import { test } from 'node:test';
import assert from 'node:assert/strict';
import { countWords, countH2, getPageQualityIssues } from '../src/lib/quality.mjs';

test('countWords excludes code blocks, paths, urls, and Sources lines', () => {
  const md =
    '# T\nHello world here.\n```js\nlots of code words do not count\n```\nSee src/a.ts and http://x.com\nSources: src/b.ts, src/c.ts';
  const n = countWords(md);
  assert.ok(n >= 4 && n <= 8, `expected ~6 prose words, got ${n}`);
});

test('countH2 counts only level-2 headings', () => {
  assert.equal(countH2('## A\n### b\n## C\ntext'), 2);
});

test('getPageQualityIssues flags a thin, unstructured page', () => {
  const issues = getPageQualityIssues('# T\nshort', { minWords: 50 });
  assert.ok(issues.some((i) => /level-2/.test(i)));
  assert.ok(issues.some((i) => /Relevant Source Files/.test(i)));
  assert.ok(issues.some((i) => /Sources:/.test(i)));
  assert.ok(issues.some((i) => /words of prose/.test(i)));
});

test('getPageQualityIssues passes a rich, grounded page', () => {
  const good =
    '# Title\n\n## Purpose and Scope\n' +
    'word '.repeat(60) +
    '\n\n## Relevant Source Files\n- `src/a.ts` matters.\n\n## Execution Flow\n' +
    'word '.repeat(60) +
    '\nSources: src/a.ts';
  assert.deepEqual(getPageQualityIssues(good, { minWords: 50 }), []);
});
