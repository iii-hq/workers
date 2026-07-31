// Scoped source-read functions exposed to the harness. Each is jailed to one
// wiki's clone directory (repoDir(wikiId)); the harness calls them via
// agent_trigger to explore the repo and cite exact line ranges. Single-reader
// surface — the orchestrator remains the single writer.
import path from 'node:path';
import fs from 'node:fs/promises';
import { repoDir } from './store.mjs';
import { inventoryRepo, readSourceFile } from './inventory.mjs';
import { lineWindow } from './harness.mjs';

const MAX_GREP_FILES = 2000;

// A harness turn calls src::list / src::grep many times per page; walking the
// clone each time is wasteful. Cache the inventory per wiki and invalidate when
// the clone changes (generation/refresh call invalidateInventory after cloning).
const invCache = new Map();
async function inventory(wikiId) {
  if (invCache.has(wikiId)) return invCache.get(wikiId);
  const v = await inventoryRepo(repoDir(wikiId));
  invCache.set(wikiId, v);
  return v;
}
export function invalidateInventory(wikiId) {
  invCache.delete(wikiId);
}

// Per-wiki read accounting: how much source material the agent actually pulled
// during a generation. Input token cost is dominated by this (plus per-turn
// context accumulation, which multiplies it). Used to measure real cost.
const readStats = new Map();
function account(wikiId, field, n) {
  const s = readStats.get(wikiId) || {
    read_calls: 0,
    read_bytes: 0,
    list_calls: 0,
    list_bytes: 0,
    grep_calls: 0,
    grep_bytes: 0,
  };
  s[field] += n;
  readStats.set(wikiId, s);
}
export function getReadStats(wikiId) {
  return readStats.get(wikiId) || null;
}
export function resetReadStats(wikiId) {
  readStats.delete(wikiId);
}

function pathEscapeError() {
  const e = new Error('path escapes repository');
  e.code = 'openwiki/path_escape';
  return e;
}
function contained(base, abs) {
  return abs === base || abs.startsWith(base + path.sep);
}

// Reject any path that escapes the wiki's clone directory, lexically AND after
// resolving symlinks (a symlink inside a clone can point outside it).
async function guard(root, rel) {
  const base = path.resolve(root);
  const abs = path.resolve(base, rel);
  if (!contained(base, abs)) throw pathEscapeError();
  try {
    const [realBase, realAbs] = await Promise.all([fs.realpath(base), fs.realpath(abs)]);
    if (!contained(realBase, realAbs)) throw pathEscapeError();
  } catch (e) {
    if (e.code === 'openwiki/path_escape') throw e;
    // ENOENT: the file does not exist; readSourceFile will surface that.
  }
  return abs;
}

export async function srcRead(wikiId, rel, from, to) {
  const dir = repoDir(wikiId);
  await guard(dir, rel);
  const { content, truncated } = await readSourceFile(dir, rel, 200_000);
  const w = lineWindow(content, from, to);
  account(wikiId, 'read_calls', 1);
  account(wikiId, 'read_bytes', w.text.length);
  return {
    path: rel,
    content: w.text,
    from: w.from,
    to: w.to,
    total_lines: w.total_lines,
    truncated: truncated || w.truncated,
  };
}

export async function srcList(wikiId, subdir) {
  const inv = await inventory(wikiId);
  let files = inv;
  if (subdir) {
    const pfx = `${String(subdir).replace(/\/+$/, '')}/`;
    files = inv.filter((e) => e.relPath.startsWith(pfx));
  }
  const truncated = files.length > 500;
  const out = files
    .slice(0, 500)
    .map((e) => ({ path: e.relPath, language: e.language, size: e.size, priority: e.priority }));
  account(wikiId, 'list_calls', 1);
  account(
    wikiId,
    'list_bytes',
    out.reduce((n, f) => n + f.path.length + 20, 0),
  );
  return { files: out, truncated };
}

export async function srcGrep(wikiId, pattern, max = 200) {
  let re;
  try {
    re = new RegExp(pattern, 'i');
  } catch {
    return { matches: [], truncated: false };
  }
  const dir = repoDir(wikiId);
  const inv = await inventory(wikiId);
  const matches = [];
  let truncated = false;
  let scanned = 0;
  for (const e of inv) {
    if (matches.length >= max || scanned >= MAX_GREP_FILES) {
      truncated = true;
      break;
    }
    if (e.language === 'text' && (e.priority ?? 0) <= 0) continue; // skip binary-ish
    scanned += 1;
    let content;
    try {
      ({ content } = await readSourceFile(dir, e.relPath, 200_000));
    } catch {
      continue;
    }
    const lines = content.split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      if (re.test(lines[i])) {
        matches.push({ path: e.relPath, line: i + 1, text: lines[i].slice(0, 300) });
        if (matches.length >= max) {
          truncated = true;
          break;
        }
      }
    }
  }
  account(wikiId, 'grep_calls', 1);
  account(
    wikiId,
    'grep_bytes',
    matches.reduce((n, m) => n + m.text.length + m.path.length, 0),
  );
  return { matches, truncated };
}
