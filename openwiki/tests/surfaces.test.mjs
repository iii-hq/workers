import { test } from 'node:test';
import assert from 'node:assert/strict';
import { heuristicAnswer, firstMeaningful, slugify } from '../src/lib/ask.mjs';
import { escLabel, heuristicMermaid } from '../src/lib/diagram.mjs';
import { buildAgentsBlock } from '../src/lib/agents_md.mjs';

test('firstMeaningful skips headings/metadata and returns the first paragraph', () => {
  assert.equal(firstMeaningful('# Title\n\n_meta_\n\nThe body here.\n\nmore'), 'The body here.');
});

test('slugify produces safe slugs', () => {
  assert.equal(slugify('What is the auth flow?'), 'what-is-the-auth-flow');
  assert.equal(slugify(''), 'answer');
});

test('heuristicAnswer stitches page excerpts', () => {
  const a = heuristicAnswer('auth', [{ slug: 'overview', title: 'Overview', excerpt: 'It authenticates.' }]);
  assert.match(a, /Overview/);
  assert.match(a, /authenticates/);
});

test('heuristicAnswer handles no matches', () => {
  assert.match(heuristicAnswer('x', []), /No wiki pages/);
});

test('escLabel neutralizes mermaid-breaking chars', () => {
  assert.equal(escLabel('a "b" [c] {d}'), "a 'b' c d");
});

test('heuristicMermaid builds a category -> pages flowchart', () => {
  const meta = { repo_name: 'o/r', categories: [{ id: 'overview', title: 'Overview' }] };
  const pages = [
    { slug: 'p1', title: 'P1', category: 'overview' },
    { slug: 'p2', title: 'P2', category: 'overview' },
  ];
  const m = heuristicMermaid(meta, pages);
  assert.match(m, /^flowchart TD/);
  assert.match(m, /ROOT\["o\/r"\]/);
  assert.match(m, /Overview/);
  assert.match(m, /P1/);
  assert.match(m, /P2/);
});

test('buildAgentsBlock lists pages, browse url, and ask hint', () => {
  const b = buildAgentsBlock(
    { id: 'w1', repo_name: 'o/r' },
    [{ slug: 'overview', title: 'Overview', category: 'overview' }],
    'http://x',
  );
  assert.match(b, /## OpenWiki/);
  assert.match(b, /Overview/);
  assert.match(b, /openwiki::ask/);
  assert.match(b, /http:\/\/x\/#\/wiki\/w1/);
});
