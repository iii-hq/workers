import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';

// Build a throwaway clone under OPENWIKI_DATA before importing src.mjs (store.mjs
// reads OPENWIKI_DATA at module load to compute repoDir).
const root = await fs.mkdtemp(path.join(os.tmpdir(), 'ow-src-'));
process.env.OPENWIKI_DATA = root;
const wikiId = 'w-test';
const repo = path.join(root, 'repos', wikiId);
await fs.mkdir(path.join(repo, 'src'), { recursive: true });
await fs.writeFile(path.join(repo, 'README.md'), '# Demo\nhello world\n');
await fs.writeFile(path.join(repo, 'src', 'a.ts'), 'export const x = 1;\nexport const y = 2;\nconsole.log(x);\n');

const src = await import('../src/lib/src.mjs');

test('srcList lists files with metadata', async () => {
  const { files } = await src.srcList(wikiId);
  const paths = files.map((f) => f.path);
  assert.ok(paths.includes('README.md'));
  assert.ok(paths.includes('src/a.ts'));
});

test('srcList honors the subdir filter', async () => {
  const { files } = await src.srcList(wikiId, 'src');
  assert.ok(files.length >= 1);
  assert.ok(files.every((f) => f.path.startsWith('src/')));
});

test('srcRead returns an inclusive line window', async () => {
  const r = await src.srcRead(wikiId, 'src/a.ts', 2, 3);
  assert.equal(r.from, 2);
  assert.equal(r.to, 3);
  assert.match(r.content, /y = 2/);
  assert.ok(r.total_lines >= 3);
});

test('srcRead rejects path traversal', async () => {
  await assert.rejects(() => src.srcRead(wikiId, '../../etc/passwd'), /escapes/);
});

test('srcGrep finds matches with line numbers', async () => {
  const { matches } = await src.srcGrep(wikiId, 'export const');
  assert.ok(matches.length >= 2);
  assert.ok(matches.every((m) => m.line > 0 && m.path && typeof m.text === 'string'));
});

test('invalidateInventory forces a re-walk so new files appear', async () => {
  await src.srcList(wikiId); // populate cache
  await fs.writeFile(path.join(repo, 'NEW.md'), 'new file');
  let { files } = await src.srcList(wikiId);
  assert.equal(
    files.some((f) => f.path === 'NEW.md'),
    false,
  ); // still cached

  src.invalidateInventory(wikiId);
  ({ files } = await src.srcList(wikiId));
  assert.equal(
    files.some((f) => f.path === 'NEW.md'),
    true,
  ); // fresh walk
});
