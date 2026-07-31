import { listPages, getPage } from './store.mjs';

const STOPWORDS = new Set(['a', 'an', 'the', 'is', 'of', 'to', 'and', 'or', 'in', 'on']);

function tokenize(s) {
  if (!s) return [];
  return String(s)
    .toLowerCase()
    .split(/\W+/)
    .filter((t) => t && !STOPWORDS.has(t));
}

function countOccurrences(tokens, target) {
  let n = 0;
  for (const t of tokens) if (t === target) n++;
  return n;
}

function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function makeSnippet(body, queryTokens) {
  const lower = body.toLowerCase();
  let idx = -1;
  for (const t of queryTokens) {
    const i = lower.indexOf(t);
    if (i !== -1 && (idx === -1 || i < idx)) idx = i;
  }
  if (idx === -1) {
    const head = body.slice(0, 200);
    return escapeHtml(head) + (body.length > 200 ? '…' : '');
  }
  const start = Math.max(0, idx - 80);
  const end = Math.min(body.length, idx + 120);
  let snip = body.slice(start, end);
  snip = escapeHtml(snip);
  if (start > 0) snip = `…${snip}`;
  if (end < body.length) snip = `${snip}…`;
  return snip;
}

export async function searchPages(wikiId, query, opts = { limit: 20 }) {
  const limit = opts?.limit ?? 20;
  const qTokens = tokenize(query);
  if (qTokens.length === 0) return [];
  const uniqQ = [...new Set(qTokens)];

  const pages = await listPages(wikiId);
  const results = [];

  for (const { slug, meta } of pages) {
    const page = await getPage(wikiId, slug);
    if (!page) continue;
    const body = page.markdown || '';
    const titleTokens = tokenize(meta.title || '');
    const catTokens = tokenize(meta.category || '');
    const bodyTokens = tokenize(body);

    let score = 0;
    const matched = [];
    for (const q of uniqQ) {
      const ct = countOccurrences(titleTokens, q);
      const cc = countOccurrences(catTokens, q);
      const cb = countOccurrences(bodyTokens, q);
      const s = 5 * ct + 2 * cc + cb;
      if (s > 0) matched.push(q);
      score += s;
    }
    if (score <= 0) continue;

    results.push({
      slug,
      title: meta.title,
      category: meta.category,
      source_paths: meta.source_paths,
      score,
      snippet: makeSnippet(body, uniqQ),
      matched,
    });
  }

  results.sort((a, b) => b.score - a.score);
  return results.slice(0, limit);
}
