// Mermaid diagram generation for a wiki. Tries a router completion for a rich
// architecture/dataflow/deps diagram; falls back to a deterministic category ->
// pages flowchart derived from the wiki structure, so it works with no provider.
import * as store from './store.mjs';
import { extractAssistantText } from './generate.mjs';

function notFound() {
  const e = new Error('wiki not found');
  e.code = 'openwiki/wiki_not_found';
  return e;
}

// Escape a mermaid node label (labels live inside "..."; quotes/brackets break it).
export function escLabel(s) {
  return String(s || '')
    .replace(/"/g, "'")
    .replace(/[[\]{}|<>]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 60);
}

export function heuristicMermaid(meta, pages) {
  const lines = ['flowchart TD', `  ROOT["${escLabel(meta.repo_name || 'repository')}"]`];
  const cats = meta.categories || [];
  const byCat = new Map();
  for (const p of pages) {
    const c = p.category || 'uncategorized';
    if (!byCat.has(c)) byCat.set(c, []);
    byCat.get(c).push(p);
  }
  let ci = 0;
  for (const [cid, ps] of byCat) {
    const cnode = `C${ci++}`;
    const ctitle = cats.find((c) => c.id === cid)?.title || cid;
    lines.push(`  ${cnode}["${escLabel(ctitle)}"]`);
    lines.push(`  ROOT --> ${cnode}`);
    let pi = 0;
    for (const p of ps.slice(0, 8)) {
      const pnode = `${cnode}_${pi++}`;
      lines.push(`  ${pnode}["${escLabel(p.title || p.slug)}"]`);
      lines.push(`  ${cnode} --> ${pnode}`);
    }
  }
  return lines.join('\n');
}

const VALID = /^\s*(flowchart|graph|sequenceDiagram|classDiagram|erDiagram|stateDiagram)/;

function stripFences(s) {
  const m = String(s || '').match(/```(?:mermaid)?\s*\n?([\s\S]*?)\n?```/i);
  return (m ? m[1] : String(s || '')).trim();
}

async function llmMermaid(worker, { meta, pages, kind, model }) {
  const pageList = pages
    .map((p) => `- ${p.title || p.slug} [${p.category || ''}]: ${(p.source_paths || []).slice(0, 4).join(', ')}`)
    .join('\n');
  const res = await worker.trigger({
    function_id: 'router::complete',
    payload: {
      model,
      system_prompt:
        'You draw Mermaid diagrams of software repositories. Output ONLY a valid Mermaid diagram, no prose, no code fences.',
      messages: [
        {
          role: 'user',
          content: `Repository: ${meta.repo_name}\nDraw a ${kind} diagram (Mermaid flowchart) from these wiki pages and their source files:\n${pageList}`,
        },
      ],
      max_output_tokens: 900,
      thinking_level: 'low',
    },
    timeoutMs: 120_000,
  });
  return stripFences(extractAssistantText(res?.message));
}

export async function makeDiagram(worker, { id, kind = 'architecture', model }) {
  const meta = await store.getWiki(id);
  if (!meta) throw notFound();
  const pages = (await store.listPages(id)).map((x) => ({ slug: x.slug, ...x.meta }));

  let mermaid = null;
  try {
    mermaid = await llmMermaid(worker, { meta, pages, kind, model: model || meta.model });
  } catch {
    mermaid = null;
  }
  if (!mermaid || !VALID.test(mermaid)) mermaid = heuristicMermaid(meta, pages);

  return { mermaid, kind };
}
