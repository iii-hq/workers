import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseLlmsTxt, candidateOrigins, docsBudget, docsHint } from '../src/lib/docs_oracle.mjs';

test('parseLlmsTxt groups links under section headings', () => {
  const txt =
    '# Docs\n\n## Get Started\n- [Install](https://x.com/install)\n- [Quickstart](https://x.com/qs)\n\n## Reference\n- [API](https://x.com/api)';
  const p = parseLlmsTxt(txt);
  assert.equal(p.links.length, 3);
  assert.equal(p.sections.length, 2);
  assert.equal(p.sections[0].title, 'Get Started');
  assert.equal(p.sections[0].links.length, 2);
});

test('candidateOrigins keeps doc hosts, skips github/badge hosts', () => {
  const readme = 'see https://docs.example.com/guide and https://github.com/o/r and https://shields.io/x';
  assert.deepEqual(candidateOrigins('', readme), ['https://docs.example.com']);
});

test('docsBudget scales with link count', () => {
  assert.equal(docsBudget(5), 18);
  assert.equal(docsBudget(50), 26);
  assert.equal(docsBudget(300), 48);
});

test('docsHint mentions sections and a page budget, empty for null', () => {
  const h = docsHint({
    source: 'x/llms.txt',
    linkCount: 100,
    sections: [
      { title: 'Learn', links: [1] },
      { title: 'Reference', links: [1] },
    ],
  });
  assert.match(h, /Learn, Reference/);
  assert.match(h, /Target about 34 pages/);
  assert.equal(docsHint(null), '');
});
