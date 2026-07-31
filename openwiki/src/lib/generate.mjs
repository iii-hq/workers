// src/lib/generate.mjs — LLM planning + page generation for OpenWiki
import { readFile } from 'node:fs/promises';

const PLAN_SYSTEM =
  'You are OpenWiki, an expert technical writer that plans documentation wikis for source code repositories. Your task is to design a small, reader-facing wiki (6–12 pages) organized by category. NEVER invent source paths. Every outline item must cite ≥1 real path from the provided inventory. Prefer conceptual pages that stitch together multiple files over one-page-per-file docs. Output STRICT JSON only.';

const PAGE_SYSTEM =
  'You are OpenWiki, a source-grounded wiki maintainer. Write ONE Markdown wiki page. Rules:\n' +
  '- Start with a level-1 heading matching the given title.\n' +
  '- Include an "Overview" section (2–4 sentences).\n' +
  '- Add topic-appropriate sections (architecture, usage, key files, workflow, notes, etc.).\n' +
  '- Cite source paths inline using backtick code spans, e.g. `src/index.ts`.\n' +
  '- Every substantive claim must be grounded in the provided source files. If a claim would require reading a file you were not shown, mark it as “needs-review” instead of guessing.\n' +
  '- Link to sibling pages using relative Markdown links: [Title](./other-slug.md).\n' +
  '- End with a "Sources" section listing every source_path with one line of what to look at.\n' +
  '- Be concise (≤ 400 lines). Prefer clarity over completeness.\n' +
  '- Do NOT emit YAML frontmatter — the wrapper will add it.';

export function extractAssistantText(message) {
  const c = message?.content;
  if (Array.isArray(c)) {
    return c
      .filter((b) => b && b.type === 'text')
      .map((b) => b.text ?? '')
      .join('');
  }
  return String(message?.content ?? '');
}

export function parseJson(text) {
  let s = String(text ?? '').trim();
  // Strip ```json ... ``` or ``` ... ``` fences
  const fence = s.match(/^```(?:json)?\s*\n?([\s\S]*?)\n?```\s*$/i);
  if (fence) s = fence[1].trim();
  try {
    return JSON.parse(s);
  } catch (_e) {
    const snip = s.slice(0, 200).replace(/\s+/g, ' ');
    throw new Error(`Model returned invalid JSON: ${snip}`);
  }
}

function extFromPath(p) {
  const m = String(p ?? '').match(/\.([A-Za-z0-9]+)$/);
  return m ? m[1].toLowerCase() : 'text';
}

async function buildKeyDocs(inventory) {
  const CAP = 40_000;
  const PER = 8_000;
  const docs = inventory.filter((e) => e.isDoc === true && (e.priority ?? 0) >= 2);
  let total = 0;
  const parts = [];
  for (const e of docs) {
    if (total >= CAP) break;
    let content = '';
    try {
      content = await readFile(e.path, 'utf8');
    } catch {
      continue;
    }
    if (content.length > PER) content = `${content.slice(0, PER)}\n...[truncated]`;
    const chunk = `### ${e.relPath}\n${content}\n`;
    if (total + chunk.length > CAP) {
      parts.push(chunk.slice(0, CAP - total));
      total = CAP;
      break;
    }
    parts.push(chunk);
    total += chunk.length;
  }
  return parts.join('');
}

function buildFileTree(inventory) {
  const sorted = [...inventory].sort((a, b) => (b.priority ?? 0) - (a.priority ?? 0));
  return sorted
    .slice(0, 200)
    .map((e) => `PRIORITY ${e.priority ?? 0}  ${e.language ?? ''}  ${e.size ?? 0}b  ${e.relPath}`)
    .join('\n');
}

async function planWikiLLM(worker, { inventory, repoName, repoUrl, model = 'claude-sonnet-4-6' }) {
  const fileTree = buildFileTree(inventory);
  const keyDocs = await buildKeyDocs(inventory);

  const userContent =
    'Plan a wiki for the repository below. Return JSON with this exact shape:\n' +
    '{\n' +
    '  "summary": string,   // 2–4 sentences describing what the repo is\n' +
    '  "categories": [ { "id": kebab-case, "title": string, "description": string } ],   // 3–7 categories\n' +
    '  "outline": [ { "slug": kebab-case, "title": string, "category": category.id, "source_paths": [relPath,...], "brief": string } ]   // 6–12 items, each with ≥1 source_path drawn from the file tree\n' +
    '}\n\n' +
    'Typical categories: overview, architecture, api, workflows, data-model, integrations, operations, decisions.\n' +
    'Ensure every outline item has category ∈ categories[].id.\n\n' +
    '---\nREPO: ' +
    repoName +
    ' (' +
    repoUrl +
    ')\n\n' +
    'FILE TREE (priority desc):\n' +
    fileTree +
    '\n\n' +
    'KEY DOCS:\n' +
    keyDocs;

  const res = await worker.trigger({
    function_id: 'router::complete',
    payload: {
      model,
      system_prompt: PLAN_SYSTEM,
      messages: [{ role: 'user', content: userContent }],
      max_output_tokens: 4000,
      thinking_level: 'medium',
    },
    timeoutMs: 120_000,
  });

  const text = extractAssistantText(res?.message);
  const parsed = parseJson(text);

  const summary = String(parsed.summary ?? '').trim();
  const categories = Array.isArray(parsed.categories) ? parsed.categories : [];
  let outline = Array.isArray(parsed.outline) ? parsed.outline : [];

  if (!summary) throw new Error('planWiki: missing summary');
  if (categories.length < 1) throw new Error('planWiki: missing categories');

  const catIds = new Set(categories.map((c) => c.id));
  const invPaths = new Set(inventory.map((e) => e.relPath));

  outline = outline.filter((item) => {
    if (!item || typeof item !== 'object') return false;
    if (!item.slug || !item.title || !item.category) {
      console.warn('[planWiki] dropping outline item missing required fields:', item?.slug ?? '<no slug>');
      return false;
    }
    if (!catIds.has(item.category)) {
      console.warn(`[planWiki] dropping "${item.slug}": unknown category "${item.category}"`);
      return false;
    }
    const paths = Array.isArray(item.source_paths) ? item.source_paths : [];
    const real = paths.filter((p) => invPaths.has(p));
    const dropped = paths.filter((p) => !invPaths.has(p));
    if (dropped.length) {
      console.warn(`[planWiki] "${item.slug}": pruning non-existent source_paths:`, dropped);
    }
    if (real.length === 0) {
      console.warn(`[planWiki] dropping "${item.slug}": no valid source_paths remain`);
      return false;
    }
    item.source_paths = real;
    return true;
  });

  if (outline.length < 3) {
    throw new Error(`planWiki: only ${outline.length} valid outline items after validation (need ≥3)`);
  }

  return { summary, categories, outline };
}

async function generatePageLLM(
  worker,
  { outlineItem, sourceReads, allSlugs, allTitles, categories, repoName, repoUrl, model = 'claude-sonnet-4-6' },
) {
  const cat = (categories ?? []).find((c) => c.id === outlineItem.category);
  const categoryTitle = cat ? cat.title : outlineItem.category;

  const siblings = (allSlugs ?? []).map((s, i) => `- ${s} — ${(allTitles ?? [])[i] ?? ''}`).join('\n');

  const sourceBlocks = (sourceReads ?? [])
    .map(
      (sr) =>
        `\n### FILE: ${sr.path}\n\n\`\`\`${extFromPath(sr.path)}\n${sr.content}${sr.truncated ? '\n...[truncated]' : ''}\n\`\`\``,
    )
    .join('\n');

  const userContent =
    'Repository: ' +
    repoName +
    ' (' +
    repoUrl +
    ')\n' +
    'Category: ' +
    categoryTitle +
    '\n' +
    'Page title: ' +
    outlineItem.title +
    '\n' +
    'Page slug: ' +
    outlineItem.slug +
    '\n' +
    'Brief: ' +
    outlineItem.brief +
    '\n\n' +
    'Sibling pages you may link to (slug — title):\n' +
    siblings +
    '\n\n' +
    'SOURCE FILES (verbatim, may be truncated):\n' +
    sourceBlocks;

  const res = await worker.trigger({
    function_id: 'router::complete',
    payload: {
      model,
      system_prompt: PAGE_SYSTEM,
      messages: [{ role: 'user', content: userContent }],
      max_output_tokens: 3500,
      thinking_level: 'low',
    },
    timeoutMs: 120_000,
  });

  let markdown = extractAssistantText(res?.message).trim();
  const wrap = markdown.match(/^```(?:markdown|md)?\s*\n([\s\S]*?)\n```\s*$/i);
  if (wrap) markdown = wrap[1].trim();

  const frontmatter = {
    title: outlineItem.title,
    slug: outlineItem.slug,
    category: outlineItem.category,
    source_paths: outlineItem.source_paths,
    last_updated: new Date().toISOString(),
    confidence: 'medium',
    status: 'current',
  };

  return { markdown, frontmatter };
}

// ------- LLM+heuristic wrappers -------
import { planWikiHeuristic, generatePageHeuristic } from './heuristic.mjs';

export async function planWiki(worker, opts) {
  try {
    return await planWikiLLM(worker, opts);
  } catch (e) {
    console.warn(`[openwiki] planWiki LLM failed (${e?.message || e}); using heuristic fallback`);
    return await planWikiHeuristic({
      inventory: opts.inventory,
      repoName: opts.repoName,
      repoUrl: opts.repoUrl,
      repoDir: opts.repoDir,
    });
  }
}

export async function generatePage(worker, opts) {
  try {
    const out = await generatePageLLM(worker, opts);
    const md = String(out?.markdown || '').trim();
    if (!md) throw new Error('LLM returned empty markdown');
    return out;
  } catch (e) {
    console.warn(
      '[openwiki] generatePage LLM failed (' +
        (e?.message || e) +
        '); using heuristic fallback for ' +
        opts?.outlineItem?.slug,
    );
    return await generatePageHeuristic(opts);
  }
}
