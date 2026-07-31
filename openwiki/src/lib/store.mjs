// iii-state backed store for openwiki. Wiki content (metadata, pages, status,
// outline, logs) lives in iii-state under `openwiki:*` scopes; cloned repos are
// ephemeral working dirs on the local filesystem (a git clone cannot live in a
// key/value store).
//
// Scaling note: `state::list` returns every value in a scope. Enumerating the
// pages scope pulls every markdown body — a large wiki can produce a multi-MB
// response that blocks the worker event loop. So pages are indexed by a single
// lightweight side-record per wiki (`openwiki:page-index` -> [{slug, meta, hash}])
// maintained on write; the hot paths (listPages, refresh mapping, content hash)
// read the index and never enumerate bodies.
import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';

const REPO_ROOT = process.env.OPENWIKI_DATA || '/tmp/openwiki-data';
export const repoDir = (id) => path.join(REPO_ROOT, 'repos', id);

let worker = null;
/** Wire the iii worker used for all state calls. Call once at startup. */
export function setWorker(c) {
  worker = c;
}

const S_WIKIS = 'openwiki:wikis';
const S_STATUS = 'openwiki:status';
const S_OUTLINE = 'openwiki:outline';
const S_LOG = 'openwiki:log';
const S_PAGE_INDEX = 'openwiki:page-index'; // wikiId -> [{ slug, meta, hash }]
const pagesScope = (id) => `openwiki:pages:${id}`;

async function sget(scope, key) {
  const res = await worker.trigger({ function_id: 'state::get', payload: { scope, key } });
  return res == null ? null : res;
}
async function sset(scope, key, value) {
  await worker.trigger({ function_id: 'state::set', payload: { scope, key, value } });
}
async function slist(scope) {
  const res = await worker.trigger({ function_id: 'state::list', payload: { scope } });
  if (Array.isArray(res)) return res;
  if (res && Array.isArray(res.values)) return res.values;
  return [];
}
async function sdel(scope, key) {
  try {
    await worker.trigger({ function_id: 'state::delete', payload: { scope, key } });
  } catch {
    /* best effort */
  }
}

function sha256(s) {
  return crypto
    .createHash('sha256')
    .update(String(s ?? ''), 'utf8')
    .digest('hex');
}

/** Ensure the local working area for repo clones exists (fs, ephemeral). */
export async function ensureRoot() {
  await fs.mkdir(path.join(REPO_ROOT, 'repos'), { recursive: true });
  await fs.mkdir(path.join(REPO_ROOT, 'tmp'), { recursive: true });
}

// ---------- wikis ----------

export async function saveWiki(id, meta) {
  await sset(S_WIKIS, id, meta);
}
export async function getWiki(id) {
  return sget(S_WIKIS, id);
}
export async function listWikis() {
  const all = await slist(S_WIKIS);
  return all.sort((a, b) => String(b.updated_at || '').localeCompare(String(a.updated_at || '')));
}

// ---------- page index (side-record; never enumerates bodies) ----------

async function getIndex(wikiId) {
  const idx = await sget(S_PAGE_INDEX, wikiId);
  return Array.isArray(idx) ? idx : [];
}
async function setIndex(wikiId, idx) {
  await sset(S_PAGE_INDEX, wikiId, idx);
}

// Serialize the page-index read-modify-write per wiki. Pages generate
// concurrently (batch writers, spawned sub-agents), so an unguarded
// getIndex -> mutate -> setIndex would lose updates.
const indexLocks = new Map();
function withIndexLock(wikiId, fn) {
  const prev = indexLocks.get(wikiId) || Promise.resolve();
  const run = prev.then(fn, fn);
  indexLocks.set(
    wikiId,
    run.catch(() => {}),
  );
  return run;
}

// Rebuild the index from the pages scope. Migration / self-heal path only —
// runs once when a wiki has pages but no index (pre-index wikis).
async function rebuildIndex(wikiId) {
  const all = await slist(pagesScope(wikiId));
  const idx = all.map((p) => ({ slug: p.slug, meta: p.meta, hash: sha256(p.markdown || '') }));
  await setIndex(wikiId, idx);
  return idx;
}

// ---------- pages ----------

export async function savePage(wikiId, slug, markdown, meta) {
  await sset(pagesScope(wikiId), slug, { slug, markdown, meta });
  await withIndexLock(wikiId, async () => {
    const idx = await getIndex(wikiId);
    const entry = { slug, meta, hash: sha256(markdown || '') };
    const i = idx.findIndex((e) => e.slug === slug);
    if (i >= 0) idx[i] = entry;
    else idx.push(entry);
    await setIndex(wikiId, idx);
  });
}

export async function getPage(wikiId, slug) {
  const p = await sget(pagesScope(wikiId), slug);
  return p ? { markdown: p.markdown, meta: p.meta } : null;
}

export async function listPages(wikiId) {
  let idx = await getIndex(wikiId);
  if (idx.length === 0) {
    // Self-heal a pre-index wiki without enumerating bodies on every call.
    const all = await slist(pagesScope(wikiId));
    if (all.length > 0) idx = await rebuildIndex(wikiId);
  }
  return idx.map((e) => ({ slug: e.slug, meta: e.meta }));
}

export async function deletePage(wikiId, slug) {
  await sdel(pagesScope(wikiId), slug);
  await withIndexLock(wikiId, async () => {
    const idx = await getIndex(wikiId);
    const next = idx.filter((e) => e.slug !== slug);
    if (next.length !== idx.length) await setIndex(wikiId, next);
  });
}

// Remove a wiki entirely: every page body, the page index, all per-wiki side
// records, the wiki meta, and the ephemeral clone. Best-effort per key so a
// partial store still clears the wiki from the list.
export async function deleteWiki(wikiId) {
  // sdel already swallows per-key errors, so the deletions below are each
  // best-effort on their own. Guard the page enumeration too: if getIndex or
  // slist throws we still want the side records and the wiki meta removed, so a
  // transient read error cannot strand the wiki in the list.
  let slugs = [];
  try {
    const idx = await getIndex(wikiId);
    slugs = idx.length ? idx.map((e) => e.slug) : (await slist(pagesScope(wikiId))).map((p) => p.slug);
  } catch {
    /* enumeration failed; still clear the records below */
  }
  for (const slug of slugs) await sdel(pagesScope(wikiId), slug);
  await sdel(S_PAGE_INDEX, wikiId);
  await sdel(S_OUTLINE, wikiId);
  await sdel(S_STATUS, wikiId);
  await sdel(S_LOG, wikiId);
  await sdel(S_WIKIS, wikiId);
  try {
    await fs.rm(repoDir(wikiId), { recursive: true, force: true });
  } catch {
    /* ephemeral */
  }
}

// Slugs whose source_paths or citations touch any of `changedPaths`. Drives
// incremental refresh — reads only the lightweight index, never page bodies.
export async function pagesForPaths(wikiId, changedPaths) {
  const want = new Set((changedPaths || []).map(String));
  if (want.size === 0) return [];
  const idx = await getIndex(wikiId);
  const hit = [];
  for (const e of idx) {
    const paths = new Set();
    for (const p of e.meta?.source_paths || []) paths.add(String(p));
    for (const c of e.meta?.citations || []) if (c?.path) paths.add(String(c.path));
    for (const p of paths)
      if (want.has(p)) {
        hit.push(e.slug);
        break;
      }
  }
  return hit;
}

// Anti-churn digest over the ordered page bodies, derived from the index hashes
// (no body enumeration). Stable for identical content regardless of write order.
export async function computeContentHash(wikiId) {
  const idx = await getIndex(wikiId);
  const parts = idx.map((e) => `${e.slug}:${e.hash}`).sort();
  return sha256(parts.join('\n'));
}

// ---------- outline / status / log ----------

export async function saveOutline(wikiId, outline) {
  await sset(S_OUTLINE, wikiId, outline);
}
export async function getOutline(wikiId) {
  return sget(S_OUTLINE, wikiId);
}

export async function updateStatus(wikiId, status) {
  await sset(S_STATUS, wikiId, { ...status, updated_at: status.updated_at || new Date().toISOString() });
}
export async function getStatus(wikiId) {
  return sget(S_STATUS, wikiId);
}

export async function appendLog(wikiId, line) {
  const ts = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');
  const prev = (await sget(S_LOG, wikiId)) || [];
  prev.push(`${ts}  ${line}`);
  if (prev.length > 500) prev.splice(0, prev.length - 500);
  await sset(S_LOG, wikiId, prev);
}
export async function getLog(wikiId) {
  return (await sget(S_LOG, wikiId)) || [];
}
