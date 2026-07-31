// Q&A over a generated wiki. Retrieves the most relevant pages, then synthesises
// a cited answer. Fast mode uses one router completion; deep mode drives the
// harness to explore both the wiki and the clone; both fall back to a heuristic
// answer (stitched page excerpts) so ask works with no provider. A good answer
// can be filed back as a new page so explorations compound into the wiki.
import crypto from 'node:crypto';
import * as store from './store.mjs';
import { searchPages } from './search.mjs';
import { extractAssistantText } from './generate.mjs';
import { awaitTurn } from './harness.mjs';

const ASK_SYSTEM =
  'You are OpenWiki answering a question about a repository using its wiki pages. ' +
  'Answer concisely in Markdown, grounded in the provided pages. Cite page titles and source paths. ' +
  'If the pages do not contain the answer, say so.';

function notFound() {
  const e = new Error('wiki not found');
  e.code = 'openwiki/wiki_not_found';
  return e;
}

export function slugify(s) {
  return (
    String(s || '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 60) || 'answer'
  );
}

// First substantive paragraph of a page (skip headings / metadata lines).
export function firstMeaningful(md, cap = 500) {
  const lines = String(md || '').split(/\r?\n/);
  const out = [];
  let started = false;
  for (const raw of lines) {
    const l = raw.trim();
    if (!started) {
      if (!l || l.startsWith('#') || l.startsWith('_') || l.startsWith('>')) continue;
      started = true;
      out.push(l);
    } else {
      if (!l || l.startsWith('#')) break;
      out.push(l);
    }
    if (out.join(' ').length > cap) break;
  }
  return out.join(' ').slice(0, cap);
}

export function heuristicAnswer(q, blocks) {
  if (!blocks.length) return `No wiki pages matched "${q}".`;
  const lines = [`The most relevant pages for "${q}":`, ''];
  for (const b of blocks) {
    lines.push(`### ${b.title || b.slug}`);
    if (b.excerpt) lines.push(b.excerpt);
    lines.push('');
  }
  return lines.join('\n').trim();
}

function dedupeCitations(citations) {
  const seen = new Set();
  const out = [];
  for (const c of citations) {
    if (!c?.path) continue;
    const key = `${c.path}:${c.start_line || ''}:${c.end_line || ''}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(c);
  }
  return out;
}

async function relevantSlugs(id, q, n = 5) {
  const results = await searchPages(id, q, { limit: n });
  if (results.length) return results.map((r) => r.slug);
  const pages = await store.listPages(id);
  return pages.slice(0, n).map((p) => p.slug);
}

async function askFastLLM(worker, { q, blocks, model }) {
  const context = blocks.map((b) => `## ${b.title || b.slug}\n${b.excerpt}`).join('\n\n');
  const res = await worker.trigger({
    function_id: 'router::complete',
    payload: {
      model,
      system_prompt: ASK_SYSTEM,
      messages: [{ role: 'user', content: `Question: ${q}\n\nWiki pages:\n${context}` }],
      max_output_tokens: 1200,
      thinking_level: 'low',
    },
    timeoutMs: 120_000,
  });
  const text = extractAssistantText(res?.message).trim();
  if (!text) throw new Error('empty answer');
  return text;
}

// Seed the agent with the pages we already retrieved so it can answer even if it
// never makes a tool call, and spell out the EXACT function signatures — agents
// otherwise guess wiki_id/query/path and every call fails.
function deepAskMessage(id, q, blocks) {
  const context = (blocks || [])
    .filter((b) => b?.excerpt)
    .map((b) => `## ${b.title || b.slug} (slug: ${b.slug})\n${b.excerpt}`)
    .join('\n\n');
  return (
    `Question: ${q}\n\n` +
    `Wiki id: ${id}\n\n` +
    `Relevant wiki pages already retrieved for you:\n${context || '(none matched — use the functions below)'}\n\n` +
    `Answer in Markdown, grounded in these pages, citing exact source file paths. ` +
    `To dig deeper, call the read functions and ALWAYS pass the wiki id as "id":\n` +
    `- openwiki::pages { id }\n` +
    `- openwiki::page { id, slug }   (slug from openwiki::pages)\n` +
    `- openwiki::search { id, q }\n` +
    `- openwiki::src::list { id, dir }\n` +
    `- openwiki::src::read { id, path }\n` +
    `- openwiki::src::grep { id, pattern }`
  );
}

async function askDeep(worker, { id, q, model, blocks }) {
  const { session_id } = await worker.trigger({
    function_id: 'harness::send',
    payload: {
      message: deepAskMessage(id, q, blocks),
      model,
      options: {
        system_prompt: ASK_SYSTEM,
        functions: {
          allow: [
            'openwiki::page',
            'openwiki::pages',
            'openwiki::search',
            'openwiki::src::read',
            'openwiki::src::list',
            'openwiki::src::grep',
          ],
        },
        max_turns: 12,
      },
    },
    timeoutMs: 30_000,
  });
  if (!session_id) throw new Error('harness::send returned no session_id');
  const result = await awaitTurn(worker, session_id, { timeoutMs: 240_000 });
  const text = typeof result === 'string' ? result : result?.answer || result?.markdown || '';
  if (!String(text).trim()) throw new Error('empty answer');
  return String(text).trim();
}

async function fileAnswer(id, q, answer, citations) {
  // Distinct questions can normalize to the same slugify output; a short
  // deterministic hash of the exact question keeps their pages separate while
  // re-asking the same question still updates its own page.
  const qHash = crypto.createHash('sha1').update(String(q)).digest('hex').slice(0, 6);
  const slug = `ask-${slugify(q)}-${qHash}`;
  const md = `# ${q}\n\n${answer}\n\n_Filed from a question on ${new Date().toISOString()}._\n`;
  await store.savePage(id, slug, md, {
    title: q.slice(0, 80),
    slug,
    category: 'answers',
    source_paths: [...new Set(citations.map((c) => c.path).filter(Boolean))],
    citations,
    last_updated: new Date().toISOString(),
    confidence: 'medium',
    status: 'current',
    generator: 'router',
  });
  return slug;
}

export async function askWiki(worker, { id, q, mode = 'fast', file_answer = false, model }) {
  const meta = await store.getWiki(id);
  if (!meta) throw notFound();
  if (!q || !String(q).trim()) return { answer: '', citations: [] };

  const slugs = await relevantSlugs(id, q, 5);
  const blocks = [];
  const citations = [];
  for (const slug of slugs) {
    const p = await store.getPage(id, slug);
    if (!p) continue;
    blocks.push({ slug, title: p.meta?.title, excerpt: firstMeaningful(p.markdown, 500) });
    for (const c of p.meta?.citations || []) citations.push(c);
  }

  let answer = null;
  try {
    answer =
      mode === 'deep'
        ? await askDeep(worker, { id, q, model: model || meta.model, blocks })
        : await askFastLLM(worker, { q, blocks, model: model || meta.model });
  } catch (e) {
    console.warn(`[openwiki] ask ${mode} tier failed: ${e?.message || e}`);
    answer = null;
  }
  // Fast mode uses router::complete; if that path is down, try the harness
  // (streaming) before dropping to the heuristic stitch.
  if (!answer && mode !== 'deep') {
    try {
      answer = await askDeep(worker, { id, q, model: model || meta.model, blocks });
    } catch (e) {
      console.warn(`[openwiki] ask deep fallback failed: ${e?.message || e}`);
      answer = null;
    }
  }
  if (!answer) answer = heuristicAnswer(q, blocks);

  const deduped = dedupeCitations(citations);
  const out = { answer, citations: deduped };
  if (file_answer) out.filed_slug = await fileAnswer(id, q, answer, deduped);
  return out;
}
