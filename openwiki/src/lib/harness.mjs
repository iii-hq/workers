// Harness-backed page generation. Instead of feeding pre-selected source files
// to one completion (generate.mjs), this drives the `harness` worker: the agent
// explores the checked-out repo through openwiki's scoped read functions
// (openwiki::src::*) and returns a validated page via a JSON output contract.
// Falls back to the router/heuristic path (generate.mjs) when the harness is
// absent or a turn fails.
import { generatePage } from './generate.mjs';
import { resolveModel } from './model.mjs';
import { getPageQualityIssues, pageRepairFeedback } from './quality.mjs';
import { PAGE_HARNESS_OUT, PLAN_HARNESS_OUT } from './schemas.mjs';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Build a GitHub blob deep-link at the pinned commit. Returns null for non-
// GitHub hosts or when the commit is unknown.
export function citationUrl(repoUrl, commit, path, from, to) {
  if (!commit || !path) return null;
  const m = String(repoUrl || '')
    .replace(/\.git$/, '')
    .match(/github\.com[:/]+([^/]+)\/([^/]+)/i);
  if (!m) return null;
  const owner = m[1];
  const repo = m[2].replace(/\/+$/, '');
  let frag = '';
  if (from) frag = `#L${from}${to && to !== from ? `-L${to}` : ''}`;
  return `https://github.com/${owner}/${repo}/blob/${commit}/${path}${frag}`;
}

// Slice a 1-indexed inclusive line window out of a file's content.
export function lineWindow(content, from, to) {
  const lines = String(content ?? '').split(/\r?\n/);
  const total = lines.length;
  if (!from && !to) return { text: String(content ?? ''), from: 1, to: total, total_lines: total, truncated: false };
  const a = Math.max(1, from || 1);
  const b = Math.min(total, to || total);
  return { text: lines.slice(a - 1, b).join('\n'), from: a, to: b, total_lines: total, truncated: a > 1 || b < total };
}

const PAGE_SYSTEM =
  'You are OpenWiki, a source-grounded technical writer producing ONE page of a repository wiki.\n' +
  'Explore the repository with these functions (always pass the given wiki id):\n' +
  '- openwiki::src::list { id, dir? } — the file tree (path, language, priority).\n' +
  '- openwiki::src::read { id, path, from?, to? } — read a file or a line window.\n' +
  '- openwiki::src::grep { id, pattern } — search file contents.\n' +
  'Read the relevant source BEFORE writing. Ground every claim in real code; never invent files, APIs, or behavior.\n\n' +
  'Write a substantial, well-structured page:\n' +
  '- Start with a single "# Title" heading.\n' +
  '- Include AT LEAST 3 "##" sections, chosen as the evidence supports: Purpose and Scope, Relevant Source Files,\n' +
  '  System-to-Code Mapping, Core Concepts, Execution Flow, Key Types and Interfaces, Configuration,\n' +
  '  Extension Points, Things to Watch When Editing, Testing Signals.\n' +
  '- ALWAYS include a "## Relevant Source Files" section: bullets naming each key file and one line on why it matters.\n' +
  '- Explain WHY the code is shaped this way, not only what it does.\n' +
  '- Ground concrete claims with visible "Sources: path/a.ts, path/b.ts" lines in the prose (copy paths exactly).\n' +
  '- Where a picture aids understanding (architecture, data flow, execution flow, a state machine, a class or module\n' +
  '  relationship), embed a Mermaid diagram INLINE as a ```mermaid fenced code block, placed in the relevant section.\n' +
  '  Keep each diagram small and valid (flowchart/sequenceDiagram/classDiagram). Do not add a diagram just to have one.\n' +
  '- Aim for 400-900 words of real explanation (code blocks and bare paths do not count).\n' +
  '- Link sibling pages inline as [Title](./slug.md).\n\n' +
  'Return JSON matching the schema: { title, markdown, citations, links, confidence, status }.\n' +
  '- markdown: the page body only (no YAML frontmatter).\n' +
  '- citations: exact { path, start_line, end_line, note } for the code each claim rests on; paths must be files you read.\n' +
  '- If a claim cannot be verified from source, set status to "needs-review".';

function buildUserPrompt({
  wikiId,
  outlineItem,
  repoName,
  repoUrl,
  categories,
  allSlugs,
  allTitles,
  feedback,
  previousMarkdown,
}) {
  const cat = (categories || []).find((c) => c.id === outlineItem.category);
  const categoryTitle = cat ? cat.title : outlineItem.category;
  const siblings = (allSlugs || []).map((s, i) => `- ${s} — ${(allTitles || [])[i] || ''}`).join('\n');
  let prompt =
    `Repository: ${repoName} (${repoUrl})\n` +
    `Wiki id (pass as "id" to every openwiki::src::* call): ${wikiId}\n` +
    `Category: ${categoryTitle}\n` +
    `Page title: ${outlineItem.title}\n` +
    `Page slug: ${outlineItem.slug}\n` +
    `Brief: ${outlineItem.brief || ''}\n` +
    `Suggested starting files: ${(outlineItem.source_paths || []).join(', ') || '(discover via openwiki::src::list)'}\n\n` +
    `Sibling pages you may link to (slug — title):\n${siblings}\n\n` +
    'Explore the source, then return the page JSON.';
  if (feedback) {
    prompt += `\n\n${feedback}`;
    if (previousMarkdown)
      prompt += `\n\nYour previous draft (improve it, keep the accurate parts):\n${previousMarkdown.slice(0, 3500)}`;
  }
  return prompt;
}

// One harness turn for a page. Runs in a named child session titled with the
// page, linked to the plan session via metadata.parent_session_id — the same
// linkage shape harness uses for real sub-agents, so the console renders each
// page as a titled child under the wiki's plan session (not an opaque s_… id).
// harness::spawn is not used here: a direct spawn call has no parent (it links
// only when dispatched from inside a running turn), so it can neither name nor
// nest. send + SessionInit does both.
async function runPageTurn(
  worker,
  {
    message,
    model,
    provider,
    parentSessionId,
    childSessionId,
    title,
    options,
    timeoutMs,
    outlineItem,
    repoUrl,
    commit,
  },
) {
  const payload = { message, model, ...(provider ? { provider } : {}), options };
  if (childSessionId) {
    payload.session_id = childSessionId;
    payload.session = {
      title: title || outlineItem.title,
      ...(parentSessionId ? { metadata: { parent_session_id: parentSessionId, depth: 1 } } : {}),
    };
  }
  const r = await worker.trigger({ function_id: 'harness::send', payload, timeoutMs: 30_000 });
  const sessionId = r?.session_id;
  if (!sessionId) throw new Error('harness::send returned no session_id');
  const result = await awaitTurn(worker, sessionId, { timeoutMs });
  return mapResult(result, { outlineItem, repoUrl, commit });
}

// A session id is any stable string; the console shows the title, not the id.
// Keep ids readable and greppable so a whole wiki's turns share a prefix.
export function sessionSlug(s) {
  return String(s || '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 48);
}
export function wikiParentSession(repoName, wikiId) {
  return `openwiki:${sessionSlug(repoName) || 'repo'}:${String(wikiId || '').slice(0, 6)}`;
}

// Poll harness::status until the turn is terminal; return its output-contract
// result. Throws on failure/timeout (best-effort stop on timeout).
export async function awaitTurn(worker, session_id, { timeoutMs = 240_000, intervalMs = 1500 } = {}) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    let s;
    try {
      s = await worker.trigger({ function_id: 'harness::status', payload: { session_id } });
    } catch (e) {
      throw new Error(`harness::status failed: ${e?.message || e}`);
    }
    const status = s?.status;
    if (status === 'completed') return s.result;
    if (status === 'failed' || status === 'cancelled') {
      throw new Error(`harness turn ${status}${s?.result_error ? `: ${s.result_error}` : ''}`);
    }
    if (Date.now() > deadline) {
      try {
        await worker.trigger({ function_id: 'harness::stop', payload: { session_id } });
      } catch {
        /* best effort */
      }
      throw new Error('harness turn timed out');
    }
    await sleep(intervalMs);
  }
}

export function mapResult(result, { outlineItem, repoUrl, commit }) {
  if (!result || typeof result !== 'object') throw new Error('harness returned no result');
  const markdown = String(result.markdown || '').trim();
  if (!markdown) throw new Error('harness returned empty markdown');
  const citations = (result.citations || [])
    .filter((c) => c?.path)
    .map((c) => {
      const url = citationUrl(repoUrl, commit, c.path, c.start_line, c.end_line);
      return { path: c.path, start_line: c.start_line, end_line: c.end_line, note: c.note, ...(url ? { url } : {}) };
    });
  const sourcePaths = [...new Set(citations.map((c) => c.path).concat(outlineItem.source_paths || []))];
  return {
    markdown,
    frontmatter: {
      title: result.title || outlineItem.title,
      slug: outlineItem.slug,
      category: outlineItem.category,
      source_paths: sourcePaths,
      citations,
      last_updated: new Date().toISOString(),
      confidence: result.confidence || 'medium',
      status: result.status || 'current',
      generator: 'harness',
    },
  };
}

// Generate one page, then run a deterministic quality gate and, if it fails,
// repair by re-prompting with the exact failing reasons (up to maxAttempts).
export async function generatePageViaHarness(worker, opts) {
  const {
    wikiId,
    outlineItem,
    repoName,
    repoUrl,
    commit,
    categories,
    allSlugs,
    allTitles,
    model,
    provider,
    parentSessionId,
    maxTurns = 12,
    timeoutMs = 240_000,
    maxAttempts = 3,
    minWords = 300,
  } = opts;
  const options = {
    system_prompt: PAGE_SYSTEM,
    output: { type: 'json', schema: PAGE_HARNESS_OUT },
    functions: { allow: ['openwiki::src::read', 'openwiki::src::list', 'openwiki::src::grep'] },
    max_turns: maxTurns,
  };

  // Name the page's session under the plan session so the console shows a
  // titled child per page instead of a fresh opaque id. Repair attempts reuse
  // the same session, so retries read as follow-up turns on that page.
  const childSessionId = parentSessionId ? `${parentSessionId}/${outlineItem.slug}` : null;
  let feedback = '';
  let previousMarkdown = '';
  let best = null;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    const message = buildUserPrompt({
      wikiId,
      outlineItem,
      repoName,
      repoUrl,
      categories,
      allSlugs,
      allTitles,
      feedback,
      previousMarkdown,
    });
    const out = await runPageTurn(worker, {
      message,
      model,
      provider,
      parentSessionId,
      childSessionId,
      title: outlineItem.title,
      options,
      timeoutMs,
      outlineItem,
      repoUrl,
      commit,
    });
    const issues = getPageQualityIssues(out.markdown, { minWords });
    best = out;
    if (!issues.length || attempt === maxAttempts) {
      out.frontmatter.quality_issues = issues.length;
      if (issues.length) out.frontmatter.status = 'needs-review';
      return out;
    }
    feedback = pageRepairFeedback(issues);
    previousMarkdown = out.markdown;
  }
  return best;
}

// Native orchestrator: ONE lead agent session runs the whole job the harness
// way. The lead researches once, decides the pages, then DELEGATES each page to a
// writer sub-agent via harness::spawn (in-turn) — that is how the harness loop
// delegates, not an injected "now spawn" directive. Each writer stores its own
// finished page by calling openwiki::write-page, so the lead never collects
// markdown (parent-side assembly of N pages was the bottleneck). The lead only
// submits the summary + navigation; the pages are already in the store.
const ORCHESTRATOR_SYSTEM =
  'You are OpenWiki. Turn a code repository into a source-grounded wiki by RESEARCHING it once and then DELEGATING each page to a writer sub-agent.\n\n' +
  'You have these functions:\n' +
  '- openwiki::src::list { id, dir? } — the file tree.\n' +
  '- openwiki::src::read { id, path, from?, to? } — read a file.\n' +
  '- openwiki::src::grep { id, pattern } — search contents.\n' +
  '- harness::spawn — start a writer sub-agent that does a focused piece of work and returns its result to you.\n\n' +
  'Steps:\n' +
  '1. RESEARCH the repository LIGHTLY with openwiki::src::* (always pass the given id). Map it first with openwiki::src::list and openwiki::src::grep, which are cheap, and openwiki::src::read only a few key files (the README, the entry point, one or two core modules) to grasp the architecture. Do NOT read the whole repo. Your own turn holds everything you read, so reading too much bloats your context and can fail the run; leave the deep, per-page file reading to the writer sub-agents. You only need enough to plan the pages and write a short brief.\n' +
  '2. PLAN the wiki as a proper documentation INDEX — the way DeepWiki or a good docs site is organized: a hierarchical table of contents that both a human and an LLM can navigate top to bottom. Decide the pages (one per real subsystem or concept) AND how they group into sections. Let the REPOSITORY drive the shape, do not impose a fixed template: a small library is a handful of pages under 2-4 sections; a large repo gets many sections, each with several pages, nested more than one level where a subsystem has sub-topics. Do NOT compress a big, varied repo into a few broad buckets, and do NOT invent sections the repo does not have. ORDER the whole index the way a reader learns the project: Overview / what-is-this FIRST, then Getting Started / Installation, then Core Concepts & Architecture, then one section per major subsystem or feature in dependency order, then API Reference, then Integration / Examples, and finally Advanced / Internals / Deployment / Performance. API Reference is never first. Scale the page count to the repo (tiny <25 files: 3-6 pages; small: 8-14; medium: 16-28; large or doc-heavy: 30-48+). For each page decide: a slug, a title, its section, the 2-6 source files that page must cover, and a one-line angle. Write a short shared "repo brief" (what the repo is, its architecture, the main modules) that you will hand to every writer so they do not re-discover it.\n' +
  '3. WRITE the wiki by spawning ONE writer sub-agent per page with harness::spawn. Put ALL the spawns in a SINGLE message so they run in parallel. For each spawn: give it session_id "<root>/<slug>"; allow it ONLY the functions openwiki::src::read, openwiki::src::list, openwiki::src::grep, and openwiki::write-page; set its output contract to json {slug, ok}. Do NOT set a model or provider on the spawns — the sub-agents automatically use yours; never name a model. Do NOT write any page yourself, and NEVER call openwiki::write-page yourself: it is only for the writer sub-agents you spawn. Writing pages inline would bloat your single turn and can fail the whole run on a large repo. Your job is to research, plan, spawn, and submit navigation, nothing more.\n' +
  '   Each writer task MUST contain: the shared repo brief; the page slug, title, and category; the specific source files to read first (it reads them with openwiki::src::* using the same id — only its focused files, not the whole repo); and this instruction: produce a substantial, source-grounded page — at least 3 "##" sections; a "## Relevant Source Files" section naming the key files; explain WHY the code is shaped this way, not only what it does; ground claims with inline path:line references; 400-900 words of real prose. CRITICAL requirements the writer must follow:\n' +
  '   - API REFERENCE: if the page documents functions, methods, options, config keys, CLI flags, or return shapes, include an "## API Reference" section that presents them as a GitHub-flavored Markdown TABLE (e.g. columns Parameter | Type | Description, and a Returns row/table). Use real signatures and types read from the source.\n' +
  '   - DIAGRAMS: wherever a picture aids understanding (architecture, data flow, execution flow, a state machine, a class or module relationship), embed a small valid Mermaid diagram INLINE as a ```mermaid fenced code block (flowchart / sequenceDiagram / classDiagram).\n' +
  '   - REFERENCES: when the writer calls openwiki::write-page, it MUST pass a citations array — one entry per source it actually used, each { path, start_line, end_line, note } with the REAL line range it read for that claim (the note is a short description). These become the clickable References list on the page, so they must be accurate. Also pass source_paths (the files the page covers).\n' +
  '   When the page is ready the writer MUST call openwiki::write-page { id, slug, title, category, markdown, source_paths, citations } to store it (write-page REJECTS pages under ~250 characters, so EVERY page — including overview, getting-started, and installation — must be genuinely substantial; ground even those in the README and entry files), then return { slug, ok:true }. Tell each writer this explicitly.\n' +
  '4. When every writer has returned ok, submit your final result. The pages are already stored, so do NOT include page bodies.\n\n' +
  "Final result JSON: { summary, navigation }. navigation IS the wiki's table of contents: a nested tree where a SECTION is { title, children:[...] } with no slug, and a PAGE is { title, slug } whose slug matches a page you delegated. Sections and pages MUST appear in the reading order from step 2 (Overview first; API Reference, Advanced, Deployment later) and may nest more than one level deep. Make it complete and well-ordered — every page you delegated appears exactly once.";

const WIKI_ORCHESTRATOR_OUT = {
  type: 'object',
  additionalProperties: true,
  required: ['navigation'],
  properties: {
    summary: { type: 'string' },
    navigation: { type: 'array', items: { type: 'object', additionalProperties: true } },
  },
};

// Drive the whole generation as one native agentic session. The writer children
// store their pages directly (openwiki::write-page); this returns only what the
// lead submits — summary + navigation — plus the root session id (children nest
// under it in the console).
export async function runOrchestrator(
  worker,
  { wikiId, repoName, repoUrl, model, docsHint = '', maxTurns = 60, timeoutMs = 600_000 },
) {
  const resolved = await resolveModel(worker, model);
  if (!resolved.resolved) throw new Error('no model available for the orchestrator');
  const root = wikiParentSession(repoName, wikiId);
  const message =
    `Repository: ${repoName} (${repoUrl})\n` +
    `Wiki id (pass as "id" to every openwiki::src::* and openwiki::write-page call, and use "${root}/<slug>" as each writer's session_id): ${wikiId}` +
    (docsHint || '') +
    '\nResearch the repo, plan the pages, spawn one writer sub-agent per page (each writer stores its page with openwiki::write-page), then submit the summary and navigation.';
  const { session_id } = await worker.trigger({
    function_id: 'harness::send',
    payload: {
      session_id: root,
      session: { title: `openwiki: ${repoName}` },
      message,
      model: resolved.model,
      ...(resolved.provider ? { provider: resolved.provider } : {}),
      options: {
        system_prompt: ORCHESTRATOR_SYSTEM,
        output: { type: 'json', schema: WIKI_ORCHESTRATOR_OUT },
        functions: {
          allow: [
            'harness::spawn',
            'openwiki::src::read',
            'openwiki::src::list',
            'openwiki::src::grep',
            'openwiki::write-page',
          ],
        },
        max_turns: maxTurns,
      },
    },
    timeoutMs: 30_000,
  });
  if (!session_id) throw new Error('orchestrator send returned no session_id');
  const result = await awaitTurn(worker, session_id, { timeoutMs });
  if (!result) throw new Error('orchestrator returned no result');
  return {
    summary: result.summary || '',
    navigation: Array.isArray(result.navigation) ? result.navigation : [],
    sessionId: session_id,
  };
}

const PLAN_SYSTEM =
  'You are OpenWiki planning a documentation wiki for a code repository.\n' +
  'Explore the repository first (always pass the given id):\n' +
  '- openwiki::src::list { id, dir? } — the file tree (path, language, priority).\n' +
  '- openwiki::src::read { id, path, from?, to? } — read a file.\n' +
  '- openwiki::src::grep { id, pattern } — search contents.\n' +
  'Read entry points, key modules, config, and docs before deciding structure.\n\n' +
  'Design a reader-facing wiki from the ACTUAL modules, subsystems, and concepts in the source.\n' +
  'Page budget scales to the repo: tiny (<25 files) 3-6 pages; small 8-14; medium 16-28; large or doc-heavy 30-48.\n' +
  'Build a real information architecture, not a flat file list:\n' +
  '- navigation is a NESTED tree, up to 3 levels: top-level folders -> optional sub-folders -> leaf pages.\n' +
  '- A folder node has a title and children and NO slug. A leaf node has a title and a slug (a real page).\n' +
  '- Start with a "Start Here" area (overview, install/getting-started), then group the rest by real subsystem or concept.\n' +
  '- Every leaf slug must appear exactly once in pages[]. Every page must reference at least one real source path.\n' +
  '- Do not create a folder that holds only one leaf; make it a page instead.\n\n' +
  'Return JSON: { summary, pages:[{slug,title,brief,source_paths}], navigation:[navNode] }.';

// Plan a wiki by having the harness explore the clone (openwiki::src::*), so the
// structure reflects the real repo rather than a heuristic template. Throws when
// the harness is unavailable; the caller falls back to the router/heuristic plan.
export async function planViaHarness(
  worker,
  {
    wikiId,
    repoName,
    repoUrl,
    model,
    docsHint = '',
    parentSessionId,
    maxTurns = 24,
    timeoutMs = 300_000,
    maxAttempts = 3,
    onRetry,
  },
) {
  const resolved = await resolveModel(worker, model);
  if (!resolved.resolved) throw new Error('no model available for harness plan');
  const base = parentSessionId || wikiParentSession(repoName, wikiId);
  const message =
    `Repository: ${repoName} (${repoUrl})\n` +
    `Wiki id (pass as "id" to every openwiki::src::* call): ${wikiId}` +
    (docsHint || '') +
    '\nExplore the repository, then return the wiki plan JSON.';
  // The plan is a long agentic turn (explore + structured output). Provider
  // streaming is the flakiest part of it ("stream ended without a terminal
  // frame"), and one failure would otherwise sink the whole generation. Retry
  // on a FRESH session each attempt so a failed turn's partial transcript never
  // bloats the retry (which would only make the next stream more likely to drop).
  let lastErr;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    const parent = attempt === 1 ? base : `${base}:r${attempt}`;
    try {
      const { session_id } = await worker.trigger({
        function_id: 'harness::send',
        payload: {
          session_id: parent,
          session: { title: `openwiki: ${repoName}` },
          message,
          model: resolved.model,
          ...(resolved.provider ? { provider: resolved.provider } : {}),
          options: {
            system_prompt: PLAN_SYSTEM,
            output: { type: 'json', schema: PLAN_HARNESS_OUT },
            functions: { allow: ['openwiki::src::read', 'openwiki::src::list', 'openwiki::src::grep'] },
            max_turns: maxTurns,
          },
        },
        timeoutMs: 30_000,
      });
      if (!session_id) throw new Error('harness::send returned no session_id for plan');
      const result = await awaitTurn(worker, session_id, { timeoutMs });
      if (!result || !Array.isArray(result.pages) || result.pages.length < 1)
        throw new Error('harness returned an empty plan');
      return {
        summary: result.summary || '',
        pages: result.pages,
        navigation: Array.isArray(result.navigation) ? result.navigation : [],
        sessionId: session_id,
      };
    } catch (e) {
      lastErr = e;
      if (attempt < maxAttempts) {
        try {
          onRetry?.(attempt, e);
        } catch {
          /* ignore */
        }
        await sleep(1500 * attempt);
      }
    }
  }
  throw lastErr || new Error('harness plan failed');
}

// Tiered page writer: harness (agentic, cited) -> router/heuristic (generate.mjs).
export async function generatePageAny(worker, opts) {
  if (opts.wikiId && opts.model && opts.useHarness !== false) {
    try {
      const out = await generatePageViaHarness(worker, opts);
      if (String(out?.markdown || '').trim()) return out;
    } catch (e) {
      if (typeof opts.onFallback === 'function') opts.onFallback(e);
    }
  }
  return generatePage(worker, opts);
}
