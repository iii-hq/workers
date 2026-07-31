// Produce the AGENTS.md / CLAUDE.md pointer block for a wiki. A coding agent that
// reads this block finds the wiki first and spends less context rediscovering the
// repo (langchain-ai/openwiki's differentiator). The wiki is hosted, not in the
// target repo, so this returns the block to paste rather than writing a file.
import * as store from './store.mjs';

function notFound() {
  const e = new Error('wiki not found');
  e.code = 'openwiki/wiki_not_found';
  return e;
}

export function buildAgentsBlock(meta, pages, baseUrl) {
  const lines = [];
  lines.push('## OpenWiki');
  lines.push('');
  lines.push(`This repository has a generated wiki for **${meta.repo_name}** (${pages.length} pages).`);
  lines.push('Read it before exploring the codebase to save context.');
  lines.push('');
  lines.push('Pages:');
  for (const p of pages) lines.push(`- ${p.title || p.slug}${p.category ? ` _(${p.category})_` : ''}`);
  lines.push('');
  if (baseUrl) lines.push(`Browse: ${baseUrl}/#/wiki/${meta.id}`);
  lines.push(`Ask: \`openwiki::ask { "id": "${meta.id}", "q": "<question>" }\``);
  return lines.join('\n');
}

export async function exportAgentsMd(_worker, { id, targets, baseUrl }) {
  const meta = await store.getWiki(id);
  if (!meta) throw notFound();
  const pages = (await store.listPages(id)).map((x) => ({ slug: x.slug, ...x.meta }));
  return { content: buildAgentsBlock(meta, pages, baseUrl), targets: targets || ['AGENTS.md', 'CLAUDE.md'] };
}
