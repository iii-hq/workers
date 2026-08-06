// OpenWiki iii worker — source-grounded wiki maintainer.
// Thin orchestrator: owns the wiki schema, page store, file->page map, and the
// UI/API surface. Git runs through the shell worker (git.mjs); persistence
// through iii-state (store.mjs); LLM through llm-router (generate.mjs).
import { registerWorker } from 'iii-sdk';
import crypto from 'node:crypto';
import fs from 'node:fs/promises';

import { cloneRepo, gitDiff, gitPull } from './lib/git.mjs';
import { inventoryRepo, readSourceFile } from './lib/inventory.mjs';
import * as store from './lib/store.mjs';
import { searchPages } from './lib/search.mjs';
import { planWiki } from './lib/generate.mjs';
import { generatePageAny, planViaHarness, runOrchestrator, wikiParentSession, citationUrl } from './lib/harness.mjs';
import { slugify } from './lib/ask.mjs';
import { resolveModel, listModels } from './lib/model.mjs';
import * as turnbus from './lib/turnbus.mjs';
import { srcRead, srcList, srcGrep, invalidateInventory, getReadStats, resetReadStats } from './lib/src.mjs';
import { lintWiki } from './lib/lint.mjs';
import { askWiki } from './lib/ask.mjs';
import { makeDiagram } from './lib/diagram.mjs';
import { exportAgentsMd } from './lib/agents_md.mjs';
import { normalizePlan, navSlugs } from './lib/nav.mjs';
import { fetchDocsIndex, docsHint as buildDocsHint } from './lib/docs_oracle.mjs';
import { pushProgress, onProgress } from './lib/progress.mjs';
import { INDEX_HTML } from './lib/ui.mjs';
import * as configuration from './lib/configuration.mjs';
import * as S from './lib/schemas.mjs';

const III_URL = process.env.III_URL || process.env.III_ENGINE_URL || 'ws://localhost:49134';
let cfg = configuration.defaults();

const worker = registerWorker(III_URL, {
  workerName: 'openwiki',
  workerDescription:
    'Source-grounded wiki maintainer: generates and maintains a categorized, interlinked markdown wiki for any git repository, and serves a browser UI + HTTP API to browse and search it.',
});

store.setWorker(worker);
store.ensureRoot().catch((e) => console.error('[openwiki] ensureRoot', e));

const now = () => new Date().toISOString();
const inflight = new Set();
// Live page count during native generation: writer sub-agents store pages via
// openwiki::write-page, which bumps this to drive the progress bar + feed.
const pagesWritten = new Map();

// Persist status AND push it to any live SSE subscriber for this wiki.
async function setStatus(wikiId, status) {
  await store.updateStatus(wikiId, status);
  pushProgress(wikiId, {
    kind: 'status',
    phase: status.phase,
    progress: status.progress,
    message: status.message,
    error: status.error,
  });
}

function err(code, message) {
  const e = new Error(message || code);
  e.code = code;
  return e;
}

// Agents (especially via the harness) routinely guess parameter names — wiki_id
// for id, query for q, path for slug. The engine does not validate request_format
// on agent tool calls, so a wrong name reaches the handler and dies with a cryptic
// state::get error. Resolve the common aliases up front and fail with a clear,
// self-correcting message instead.
function wikiIdOf(a) {
  return a?.id || a?.wiki_id || a?.wikiId || a?.wikiID;
}
function needId(a, fn) {
  const id = wikiIdOf(a);
  if (!id)
    throw err(
      'openwiki/bad_request',
      `${fn} requires the wiki id as "id" (you passed: ${Object.keys(a || {}).join(', ') || 'nothing'})`,
    );
  return id;
}

async function readRepoReadme(dir) {
  for (const name of ['README.md', 'README.mdx', 'readme.md', 'Readme.md']) {
    try {
      return await fs.readFile(`${dir}/${name}`, 'utf8');
    } catch {
      /* try next */
    }
  }
  return '';
}

// Drop nav leaves whose page never landed (a writer failed), and any section
// left empty, so the index never links to a missing page.
function pruneNav(nodes, valid) {
  const out = [];
  for (const n of nodes || []) {
    if (n.children?.length) {
      const kids = pruneNav(n.children, valid);
      if (kids.length) out.push({ ...n, children: kids });
      else if (n.slug && valid.has(n.slug)) out.push({ title: n.title, slug: n.slug });
    } else if (n.slug && valid.has(n.slug)) {
      out.push(n);
    }
  }
  return out;
}

// Group stored pages into a category nav tree. Used when writers stored pages
// but the lead never submitted a navigation (its final turn failed).
function navFromPages(pages) {
  const byCat = new Map();
  for (const p of pages) {
    const cat = p.meta?.category || 'Pages';
    if (!byCat.has(cat)) byCat.set(cat, []);
    byCat.get(cat).push({ title: p.meta?.title || p.slug, slug: p.slug });
  }
  return [...byCat.entries()].map(([title, children]) => ({ title, children }));
}

function inferRepoName(url) {
  return (
    String(url || '')
      .replace(/\.git$/, '')
      .replace(/\/+$/, '')
      .split(/[/:]/)
      .filter(Boolean)
      .slice(-2)
      .join('/') || url
  );
}

// Write a set of outline items to pages. Shared by full generation and
// incremental refresh; `fullOutline` supplies sibling links for cross-refs so a
// partial refresh still links to the whole wiki.
async function writePages(
  wikiId,
  {
    dir,
    itemsToWrite,
    fullOutline,
    categories,
    repoName,
    repoUrl,
    model,
    commit,
    parentSessionId,
    progressBase = 0.3,
    progressSpan = 0.65,
  },
) {
  // The native agentic path is runOrchestrator (see runGeneration). This runs
  // only as the fallback for the router / heuristic tiers, synchronously in
  // bounded parallel.
  const resolved = await resolveModel(worker, model);
  return writePagesSync(wikiId, {
    dir,
    itemsToWrite,
    fullOutline,
    categories,
    repoName,
    repoUrl,
    model,
    commit,
    parentSessionId,
    resolved,
    progressBase,
    progressSpan,
  });
}

async function writePagesSync(
  wikiId,
  {
    dir,
    itemsToWrite,
    fullOutline,
    categories,
    repoName,
    repoUrl,
    model,
    commit,
    parentSessionId,
    resolved,
    progressBase = 0.3,
    progressSpan = 0.65,
  },
) {
  const allSlugs = fullOutline.map((o) => o.slug);
  const allTitles = fullOutline.map((o) => o.title);
  const total = itemsToWrite.length || 1;
  const step = Math.max(1, Number(cfg.max_parallel) || 1);
  let done = 0;

  for (let i = 0; i < itemsToWrite.length; i += step) {
    const batch = itemsToWrite.slice(i, i + step);
    await Promise.all(
      batch.map(async (item) => {
        const reads = [];
        for (const p of item.source_paths || []) {
          try {
            reads.push(await readSourceFile(dir, p, 40_000));
          } catch (e) {
            reads.push({ path: p, content: `(unreadable: ${e.message})`, truncated: false });
          }
        }
        try {
          const { markdown, frontmatter } = await generatePageAny(worker, {
            wikiId,
            outlineItem: item,
            sourceReads: reads,
            allSlugs,
            allTitles,
            categories,
            repoName,
            repoUrl,
            commit,
            model: resolved.model || model,
            provider: resolved.provider,
            useHarness: resolved.resolved,
            parentSessionId,
            onFallback: (err) => store.appendLog(wikiId, `harness fallback for ${item.slug}: ${err?.message || err}`),
          });
          await store.savePage(wikiId, item.slug, markdown, frontmatter);
          await store.appendLog(wikiId, `Wrote ${item.slug} — ${item.title}`);
          pushProgress(wikiId, { kind: 'page', slug: item.slug, title: item.title });
        } catch (e) {
          await store.appendLog(wikiId, `FAILED ${item.slug}: ${e?.message || e}`);
          await store.savePage(
            wikiId,
            item.slug,
            `# ${item.title}\n\n> Generation failed: ${e?.message || e}\n\n_Source paths_: ${(item.source_paths || []).map((p) => `\`${p}\``).join(', ') || '(none)'}\n`,
            {
              title: item.title,
              slug: item.slug,
              category: item.category,
              source_paths: item.source_paths || [],
              last_updated: now(),
              confidence: 'low',
              status: 'needs-review',
            },
          );
        }
        done += 1;
        await setStatus(wikiId, {
          phase: 'generating',
          progress: progressBase + progressSpan * (done / total),
          message: `Generated ${done}/${total} pages`,
          pages_done: done,
          pages_total: total,
          updated_at: now(),
        });
      }),
    );
  }
}

// ---------- Full generation ----------

async function runGeneration(wikiId, { repoUrl, model, ref, steer }) {
  inflight.add(wikiId);
  resetReadStats(wikiId);
  pagesWritten.set(wikiId, 0);
  const started = now();
  try {
    await store.appendLog(wikiId, `Starting generation for ${repoUrl} (model=${model})`);
    await setStatus(wikiId, { phase: 'cloning', progress: 0.05, message: 'Cloning repository', updated_at: now() });

    const dir = store.repoDir(wikiId);
    await fs.rm(dir, { recursive: true, force: true });
    const { commit, name } = await cloneRepo(worker, repoUrl, dir, ref);
    invalidateInventory(wikiId);
    // Persist the cloned commit + name now so openwiki::write-page can build
    // pinned GitHub blob URLs for each page's citations while writers run.
    const base = await store.getWiki(wikiId);
    if (base) await store.saveWiki(wikiId, { ...base, repo_name: name, commit, updated_at: now() });

    await setStatus(wikiId, {
      phase: 'inventorying',
      progress: 0.15,
      message: 'Reading source files',
      updated_at: now(),
    });
    const inventory = await inventoryRepo(dir);
    await store.appendLog(wikiId, `Inventoried ${inventory.length} files.`);

    await setStatus(wikiId, {
      phase: 'planning',
      progress: 0.25,
      message: 'Exploring repo and planning structure',
      updated_at: now(),
    });
    let dHint = '';
    try {
      const readme = await readRepoReadme(dir);
      const docsIndex = await fetchDocsIndex(worker, { repoUrl, readme, repoDir: dir });
      if (docsIndex) {
        dHint = buildDocsHint(docsIndex);
        await store.appendLog(wikiId, `docs oracle: ${docsIndex.linkCount} topics (${docsIndex.source})`);
      }
    } catch {
      /* oracle is optional */
    }
    // Native path: ONE lead agent researches the repo, plans the pages, and
    // spawns one writer sub-agent per page (harness::spawn) — the harness way.
    // Each writer stores its OWN page via openwiki::write-page, so the lead only
    // returns the summary + navigation; the pages are already on disk here.
    let pages = null;
    let navigation = [];
    let summary = '';
    let categories = [];
    // Live progress: writers store pages via openwiki::write-page (which streams
    // the page + advances the bar). Spawns are only visible on turn-started, so
    // subscribe that to stream the "spawned a writer" feed as the lead delegates.
    const orchRoot = wikiParentSession(name, wikiId);
    const offLive = turnbus.register(orchRoot, {
      onSpawn: (childId) => {
        pushProgress(wikiId, { kind: 'spawn', slug: String(childId).split('/').pop() || 'page' });
      },
    });
    let orch = null;
    try {
      await setStatus(wikiId, {
        phase: 'generating',
        progress: 0.3,
        message: 'Agent researching and spawning writers',
        updated_at: now(),
      });
      orch = await runOrchestrator(worker, { wikiId, repoName: name, repoUrl, model, docsHint: dHint });
    } catch (e) {
      await store.appendLog(wikiId, `lead did not finish cleanly (${e?.message || e})`);
    } finally {
      offLive();
    }
    // The writers store pages directly, so use whatever landed even if the lead
    // failed to submit its final nav (a large final turn can time out). Only fall
    // back to the router plan when nothing was written at all.
    const stored = await store.listPages(wikiId);
    if (stored.length) {
      navigation =
        orch && Array.isArray(orch.navigation) && orch.navigation.length ? orch.navigation : navFromPages(stored);
      summary = orch?.summary || '';
      // categories is derived below from the PRUNED navigation (post pruneNav),
      // so it is intentionally not computed here.
      pages = stored;
      await store.appendLog(
        wikiId,
        `Using ${stored.length} writer-stored page(s)${orch ? '' : ' (lead did not submit nav; grouped by category)'}.`,
      );
    }

    if (pages) {
      // Pages are already stored by the writers; build the outline from what
      // landed and prune the index so it never links to a page a writer failed to
      // produce (with the write-page thin-guard, stored pages are all substantial).
      const items = pages.map((p) => ({
        slug: p.slug,
        title: p.meta?.title || p.slug,
        category: p.meta?.category || '',
        source_paths: p.meta?.source_paths || [],
      }));
      const haveSlugs = new Set(items.map((i) => i.slug));
      const missing = navSlugs(navigation).filter((s) => !haveSlugs.has(s));
      if (missing.length)
        await store.appendLog(
          wikiId,
          `pruned ${missing.length} unwritten page(s) from the index: ${missing.join(', ')}`,
        );
      navigation = pruneNav(navigation, haveSlugs);
      categories = navigation.map((l1) => ({ id: l1.title, title: l1.title }));
      await store.saveOutline(wikiId, { navigation, categories, items });
    } else {
      // Fallback: two-phase plan + parallel writers (no harness model, or the
      // orchestrator failed). Keeps openwiki working without the agentic path.
      let planned = null;
      try {
        planned = await planViaHarness(worker, { wikiId, repoName: name, repoUrl, model, docsHint: dHint });
      } catch (e) {
        await store.appendLog(wikiId, `harness plan fallback (${e?.message || e})`);
      }
      if (!planned)
        planned = await planWiki(worker, { inventory, repoName: name, repoUrl, model, repoDir: dir, steer });
      const parentSessionId = planned?.sessionId;
      const norm = normalizePlan(planned);
      summary = norm.summary;
      navigation = norm.navigation;
      const invPaths = new Set(inventory.map((e) => e.relPath));
      const outline = norm.outline.map((item) => ({
        ...item,
        source_paths: (item.source_paths || []).filter((p) => invPaths.has(p)),
      }));
      categories = navigation.map((l1) => ({ id: l1.title, title: l1.title }));
      await store.saveOutline(wikiId, { navigation, categories, items: outline });
      await store.saveWiki(wikiId, {
        id: wikiId,
        repo_url: repoUrl,
        repo_name: name,
        ref: ref || '',
        commit,
        created_at: started,
        updated_at: now(),
        page_count: outline.length,
        category_count: categories.length,
        categories,
        navigation,
        summary,
        model,
        steer: steer || undefined,
        generating: true,
      });
      await writePages(wikiId, {
        dir,
        itemsToWrite: outline,
        fullOutline: outline,
        categories,
        repoName: name,
        repoUrl,
        model,
        commit,
        parentSessionId,
      });
      pages = outline;
    }

    const content_hash = await store.computeContentHash(wikiId);
    // Preserve the auto-refresh cadence across a (re)generation (this saveWiki
    // rebuilds meta from scratch), defaulting a brand-new wiki to the config
    // default. Then (re)register its cron trigger.
    const prior = await store.getWiki(wikiId);
    const refresh_schedule = prior?.refresh_schedule || cfg.refresh_default || 'off';
    await store.saveWiki(wikiId, {
      id: wikiId,
      repo_url: repoUrl,
      repo_name: name,
      ref: ref || '',
      commit,
      created_at: started,
      updated_at: now(),
      page_count: pages.length,
      category_count: categories.length,
      categories,
      navigation,
      summary,
      model,
      steer: steer || undefined,
      generating: false,
      content_hash,
      refresh_schedule,
      last_refresh_at: prior?.last_refresh_at || undefined,
    });
    applyWikiSchedule(wikiId, refresh_schedule);
    await setStatus(wikiId, { phase: 'ready', progress: 1, message: 'Wiki ready', updated_at: now() });
    await store.appendLog(wikiId, 'Wiki ready.');
    try {
      const { issues } = await lintWiki(wikiId);
      if (issues.length) await store.appendLog(wikiId, `lint: ${issues.length} issue(s) flagged`);
    } catch (e) {
      await store.appendLog(wikiId, `lint skipped: ${e?.message || e}`);
    }
  } catch (e) {
    console.error('[openwiki] generation error', e);
    await store.appendLog(wikiId, `ERROR: ${e?.stack || e?.message || e}`);
    await setStatus(wikiId, { phase: 'error', progress: 0, error: String(e?.message || e), updated_at: now() });
  } finally {
    inflight.delete(wikiId);
    pagesWritten.delete(wikiId);
  }
}

async function startWiki(repoUrl, model, ref, steer) {
  if (!repoUrl || typeof repoUrl !== 'string') throw err('openwiki/repo_not_found', 'repo_url required');
  const wikiId = crypto.randomUUID();
  const chosen = model || cfg.model;
  await store.saveWiki(wikiId, {
    id: wikiId,
    repo_url: repoUrl,
    repo_name: inferRepoName(repoUrl),
    ref: ref || '',
    commit: '',
    created_at: now(),
    updated_at: now(),
    page_count: 0,
    category_count: 0,
    categories: [],
    summary: '',
    model: chosen,
    steer: steer || undefined,
    generating: true,
    refresh_schedule: cfg.refresh_default || 'off',
  });
  await setStatus(wikiId, { phase: 'queued', progress: 0, message: 'Queued', updated_at: now() });
  setImmediate(() => {
    runGeneration(wikiId, { repoUrl, model: chosen, ref, steer }).catch((e) => console.error(e));
  });
  return { wiki_id: wikiId, status: 'queued' };
}

// ---------- Incremental refresh ----------

// Regenerate only the affected pages, then gate on a content hash so an
// identical result does not churn the wiki's updated_at (langchain-ai/openwiki
// anti-churn: git-head gate + content-hash gate).
async function runRefresh(wikiId, { dir, itemsToWrite, outline, meta, newCommit, prevHash }) {
  inflight.add(wikiId);
  try {
    await setStatus(wikiId, {
      phase: 'generating',
      progress: 0.3,
      message: `Refreshing ${itemsToWrite.length} pages`,
      updated_at: now(),
    });
    await writePages(wikiId, {
      dir,
      itemsToWrite,
      fullOutline: outline.items || [],
      categories: outline.categories || meta.categories || [],
      repoName: meta.repo_name,
      repoUrl: meta.repo_url,
      model: meta.model || cfg.model,
      commit: newCommit,
    });
    const content_hash = await store.computeContentHash(wikiId);
    const churned = content_hash !== prevHash;
    await store.saveWiki(wikiId, {
      ...meta,
      commit: newCommit,
      content_hash,
      page_count: (outline.items || []).length,
      updated_at: now(),
    });
    await setStatus(wikiId, {
      phase: 'ready',
      progress: 1,
      message: churned ? 'Refresh complete' : 'No content change',
      updated_at: now(),
    });
    await store.appendLog(
      wikiId,
      `refresh: regenerated ${itemsToWrite.length} pages (content ${churned ? 'changed' : 'unchanged'})`,
    );
  } catch (e) {
    await store.appendLog(wikiId, `refresh error: ${e?.message || e}`);
    await setStatus(wikiId, { phase: 'error', progress: 0, error: String(e?.message || e), updated_at: now() });
  } finally {
    inflight.delete(wikiId);
  }
}

async function refreshWiki(wikiId) {
  const meta = await store.getWiki(wikiId);
  if (!meta) throw err('openwiki/wiki_not_found', 'wiki not found');

  // One refresh (or generation) per wiki at a time: concurrent runs would race
  // the same clone directory through gitPull/cloneRepo. The marker is held
  // until the scheduled runGeneration/runRefresh takes ownership (each re-adds
  // it and deletes it in its own finally).
  if (inflight.has(wikiId)) {
    return { wiki_id: wikiId, refresh: 'in_progress', changed: [], pages_affected: [] };
  }
  inflight.add(wikiId);
  let handedOff = false;
  try {
    return await doRefresh(wikiId, meta, () => {
      handedOff = true;
    });
  } finally {
    if (!handedOff) inflight.delete(wikiId);
  }
}

async function doRefresh(wikiId, meta, markHandedOff) {
  // Stamp the refresh clock up front so the scheduled due-check advances even
  // when this run finds nothing to do or kicks off an async rebuild.
  meta.last_refresh_at = now();
  await store.saveWiki(wikiId, meta);

  const dir = store.repoDir(wikiId);
  const prevCommit = meta.commit || '';
  let newCommit = null;

  // Ensure a clone exists and is current.
  const stat = await fs.stat(dir).catch(() => null);
  if (stat?.isDirectory()) {
    newCommit = await gitPull(worker, dir);
    if (!newCommit) {
      await fs.rm(dir, { recursive: true, force: true });
      ({ commit: newCommit } = await cloneRepo(worker, meta.repo_url, dir, meta.ref));
    }
  } else {
    ({ commit: newCommit } = await cloneRepo(worker, meta.repo_url, dir, meta.ref));
  }
  invalidateInventory(wikiId);

  // Anti-churn: HEAD unchanged -> nothing to do.
  if (prevCommit && newCommit && prevCommit === newCommit) {
    await store.appendLog(wikiId, 'refresh: HEAD unchanged');
    return { wiki_id: wikiId, refresh: 'up_to_date', changed: [], pages_affected: [] };
  }

  // No prior commit / no pages -> full build.
  const priorPages = await store.listPages(wikiId);
  const fullRebuild = () => {
    markHandedOff();
    setImmediate(() => {
      runGeneration(wikiId, {
        repoUrl: meta.repo_url,
        model: meta.model || cfg.model,
        ref: meta.ref,
        steer: meta.steer,
      }).catch((e) => console.error(e));
    });
    return { wiki_id: wikiId, refresh: 'regenerating', changed: [], pages_affected: [] };
  };
  if (!prevCommit || priorPages.length === 0) return fullRebuild();

  // Diff prev..new, map changed files to affected pages, regenerate only those.
  // A failed diff (e.g. the previous commit was pruned from the clone) means
  // the change set is unknown; rebuild rather than report up_to_date.
  let changed = [];
  try {
    changed = await gitDiff(worker, dir, prevCommit, newCommit);
  } catch (e) {
    await store.appendLog(wikiId, `refresh diff failed (${e?.message || e}); treating as unknown changes`);
    return fullRebuild();
  }
  const changedPaths = changed.map((c) => c.path);
  const affected = await store.pagesForPaths(wikiId, changedPaths);
  await store.appendLog(wikiId, `refresh: ${changed.length} changed paths -> ${affected.length} affected pages`);

  if (affected.length === 0) {
    await store.saveWiki(wikiId, { ...meta, commit: newCommit, updated_at: now() });
    return { wiki_id: wikiId, refresh: 'up_to_date', changed, pages_affected: [] };
  }

  const outline = (await store.getOutline(wikiId)) || { items: [], categories: meta.categories || [] };
  const itemsToWrite = (outline.items || []).filter((o) => affected.includes(o.slug));
  const prevHash = meta.content_hash || (await store.computeContentHash(wikiId));
  markHandedOff();
  setImmediate(() => {
    runRefresh(wikiId, { dir, itemsToWrite, outline, meta, newCommit, prevHash }).catch((e) => console.error(e));
  });
  return { wiki_id: wikiId, refresh: 'regenerating', changed, pages_affected: itemsToWrite.map((i) => i.slug) };
}

// ---------- iii functions ----------

worker.registerFunction(
  'openwiki::generate',
  async ({ repo_url, model, ref, steer }) => startWiki(repo_url, model, ref, steer),
  {
    description:
      'Start generating a source-grounded wiki for a git repository URL. Returns immediately with { wiki_id, status }; poll openwiki::status.',
    request_format: S.GENERATE_REQ,
    response_format: S.GENERATE_RES,
  },
);

worker.registerFunction(
  'openwiki::status',
  async (a) => (await store.getStatus(wikiIdOf(a))) || { phase: 'unknown', progress: 0, updated_at: now() },
  { description: 'Poll generation status for a wiki id.', request_format: S.STATUS_REQ, response_format: S.STATUS_RES },
);

worker.registerFunction('openwiki::wikis', async () => ({ wikis: await store.listWikis() }), {
  description: 'List all wikis generated by this worker.',
  request_format: S.WIKIS_REQ,
  response_format: S.WIKIS_RES,
});

worker.registerFunction(
  'openwiki::models',
  async () => ({ models: await listModels(worker), default_model: cfg.model }),
  {
    description:
      "Models available via llm-router (for the UI's model picker), plus the configured default. Empty when no provider is configured.",
    request_format: S.MODELS_REQ,
    response_format: S.MODELS_RES,
  },
);

worker.registerFunction(
  'openwiki::wiki',
  async (a) => {
    const id = needId(a, 'openwiki::wiki');
    const m = await store.getWiki(id);
    if (!m) throw err('openwiki/wiki_not_found', 'wiki not found');
    return m;
  },
  { description: "Fetch a single wiki's metadata.", request_format: S.WIKI_REQ, response_format: S.WIKI_RES },
);

worker.registerFunction(
  'openwiki::pages',
  async (a) => {
    const id = needId(a, 'openwiki::pages');
    return { pages: (await store.listPages(id)).map((x) => ({ slug: x.slug, ...x.meta })) };
  },
  {
    description: 'List all pages of a wiki. Params: { id }.',
    request_format: S.PAGES_REQ,
    response_format: S.PAGES_RES,
  },
);

worker.registerFunction(
  'openwiki::page',
  async (a) => {
    const id = needId(a, 'openwiki::page');
    const raw = a.slug || a.page || a.path || a.title;
    if (!raw)
      throw err('openwiki/bad_request', 'openwiki::page requires a page "slug" (list them with openwiki::pages)');
    const slug = slugify(String(raw));
    const p = await store.getPage(id, slug);
    if (!p) {
      const have = (await store.listPages(id)).map((x) => x.slug);
      throw err('openwiki/page_not_found', `page "${slug}" not found; available slugs: ${have.join(', ') || '(none)'}`);
    }
    return { slug, ...p.meta, markdown: p.markdown };
  },
  {
    description: 'Get a single wiki page (markdown body + metadata). Params: { id, slug } (slug from openwiki::pages).',
    request_format: S.PAGE_REQ,
    response_format: S.PAGE_RES,
  },
);

worker.registerFunction(
  'openwiki::search',
  async (a) => {
    const id = needId(a, 'openwiki::search');
    return { results: await searchPages(id, a.q || a.query || '') };
  },
  {
    description: 'Search pages of a wiki by keyword. Params: { id, q }.',
    request_format: S.SEARCH_REQ,
    response_format: S.SEARCH_RES,
  },
);

worker.registerFunction('openwiki::refresh', async (a) => refreshWiki(needId(a, 'openwiki::refresh')), {
  description: 'Pull the repo and regenerate only the pages whose source changed (incremental).',
  request_format: S.WIKI_REQ,
  response_format: S.REFRESH_RES,
});

worker.registerFunction(
  'openwiki::delete',
  async (a) => {
    const id = needId(a, 'openwiki::delete');
    if (inflight.has(id)) throw err('openwiki/generating', 'wiki is generating; stop it before deleting');
    applyWikiSchedule(id, 'off'); // tear down any auto-refresh trigger for this wiki
    await store.deleteWiki(id);
    invalidateInventory(id);
    resetReadStats(id);
    return { id, deleted: true };
  },
  {
    description: 'Delete a wiki and all its pages. Params: { id }.',
    request_format: S.WIKI_REQ,
    response_format: {
      type: 'object',
      additionalProperties: false,
      required: ['id', 'deleted'],
      properties: { id: { type: 'string' }, deleted: { type: 'boolean' } },
    },
  },
);

worker.registerFunction('openwiki::lint', async (a) => lintWiki(needId(a, 'openwiki::lint')), {
  description: 'Validate every page citation against the clone and flag thin pages.',
  request_format: S.LINT_REQ,
  response_format: S.LINT_RES,
});

worker.registerFunction(
  'openwiki::gen-stats',
  async (a) => {
    const id = needId(a, 'openwiki::gen-stats');
    const reads = getReadStats(id) || {};
    const pages = await store.listPages(id);
    let output_bytes = 0;
    for (const p of pages) {
      const pg = await store.getPage(id, p.slug);
      if (pg) output_bytes += (pg.markdown || '').length;
    }
    return { reads, page_count: pages.length, output_bytes };
  },
  {
    description: 'Measurement: source bytes the agent read and page bytes produced for a generation.',
    request_format: S.WIKI_REQ,
    response_format: {
      type: 'object',
      additionalProperties: true,
      properties: { page_count: { type: 'integer' }, output_bytes: { type: 'integer' } },
    },
  },
);

// ---------- Scoped source readers (the harness's exploration tools) ----------
// Each is jailed to one wiki's clone; the page-writer harness calls these via
// agent_trigger to explore the repo and cite exact line ranges.

// Surface the harness's file exploration as live activity. During planning
// (one long opaque plan turn) there is no page progress, so without this the UI
// sits static and reads as frozen. Each read/list/grep pushes a cheap activity
// frame the panel shows as the current line ("reading src/queue.js").
worker.registerFunction(
  'openwiki::src::read',
  async (a) => {
    const id = needId(a, 'openwiki::src::read');
    const path = a.path || a.file || a.filename;
    if (!path) throw err('openwiki/bad_request', 'openwiki::src::read requires a file "path"');
    pushProgress(id, { kind: 'activity', op: 'read', path });
    return srcRead(id, path, a.from, a.to);
  },
  {
    description:
      "Read a file (optional 1-indexed line window) from a wiki's cloned repo. Params: { id, path, from?, to? }.",
    request_format: S.SRC_READ_REQ,
    response_format: S.SRC_READ_RES,
  },
);

worker.registerFunction(
  'openwiki::src::list',
  async (a) => {
    const id = needId(a, 'openwiki::src::list');
    const dir = a.dir || a.path || '';
    pushProgress(id, { kind: 'activity', op: 'list', path: dir || '.' });
    return srcList(id, dir);
  },
  {
    description: "List files (path, language, priority) in a wiki's cloned repo. Params: { id, dir? }.",
    request_format: S.SRC_LIST_REQ,
    response_format: S.SRC_LIST_RES,
  },
);

worker.registerFunction(
  'openwiki::src::grep',
  async (a) => {
    const id = needId(a, 'openwiki::src::grep');
    const pattern = a.pattern || a.query || a.q;
    if (!pattern) throw err('openwiki/bad_request', 'openwiki::src::grep requires a "pattern"');
    pushProgress(id, { kind: 'activity', op: 'grep', path: pattern });
    return srcGrep(id, pattern, a.max);
  },
  {
    description: "Search file contents in a wiki's cloned repo. Params: { id, pattern, max? }.",
    request_format: S.SRC_GREP_REQ,
    response_format: S.SRC_GREP_RES,
  },
);

// A page-writer sub-agent stores its finished page here directly, so the parent
// never collects the markdown (that assembly was the bottleneck). Guarded to a
// wiki that is actively generating; the concurrent index write is serialized in
// store.savePage. Streams a live page event.
worker.registerFunction(
  'openwiki::write-page',
  async ({ id, slug, title, category, markdown, source_paths, citations }) => {
    const meta = await store.getWiki(id);
    if (!meta?.generating) throw err('openwiki/not_generating', 'wiki is not generating; write-page refused');
    const s = slugify(slug || title || 'page');
    // Reject thin/empty pages so an under-delivering writer cannot store a blank
    // page (that landed empty "overview"/"installation" pages in the index). The
    // writer sees this error and can retry with real content within its turns.
    const body = String(markdown || '').trim();
    if (body.length < 250)
      throw err(
        'openwiki/thin_page',
        `page "${s}" is too thin (${body.length} chars). Write a substantial, source-grounded page (at least 3 "##" sections and 400+ words) before calling write-page.`,
      );
    // Build the References rail: turn the writer's citations (and any bare
    // source_paths) into pinned GitHub blob URLs at the cloned commit, so the UI
    // renders a References list and makes each source file a clickable deep-link.
    const cited = (Array.isArray(citations) ? citations : []).filter((c) => c?.path);
    const paths = [...new Set(cited.map((c) => c.path).concat(source_paths || []))];
    const withUrl = cited.map((c) => {
      const url = citationUrl(meta.repo_url, meta.commit, c.path, c.start_line, c.end_line);
      return {
        path: c.path,
        ...(c.start_line ? { start_line: c.start_line } : {}),
        ...(c.end_line ? { end_line: c.end_line } : {}),
        ...(c.note ? { note: c.note } : {}),
        ...(url ? { url } : {}),
      };
    });
    for (const p of paths) {
      if (withUrl.some((c) => c.path === p)) continue;
      const url = citationUrl(meta.repo_url, meta.commit, p);
      withUrl.push({ path: p, ...(url ? { url } : {}) });
    }
    const frontmatter = {
      title: title || s,
      slug: s,
      category: category || '',
      source_paths: paths,
      citations: withUrl,
      last_updated: now(),
      confidence: 'medium',
      status: 'current',
      generator: 'harness',
    };
    await store.savePage(id, s, body, frontmatter);
    const n = (pagesWritten.get(id) || 0) + 1;
    pagesWritten.set(id, n);
    await store.appendLog(id, `Wrote ${s} — ${title || s}`);
    pushProgress(id, { kind: 'page', slug: s, title: title || s });
    await setStatus(id, {
      phase: 'generating',
      progress: Math.min(0.92, 0.35 + n * 0.06),
      message: `Writers stored ${n} page(s)`,
      pages_done: n,
      updated_at: now(),
    });
    return { slug: s, ok: true };
  },
  {
    description: 'A page-writer sub-agent stores its finished page (markdown + metadata) for a generating wiki.',
    request_format: S.WRITE_PAGE_REQ,
    response_format: S.WRITE_PAGE_RES,
  },
);

// ---------- Ask / diagram / export ----------

worker.registerFunction(
  'openwiki::ask',
  async (a) =>
    askWiki(worker, {
      id: needId(a, 'openwiki::ask'),
      q: a.q,
      mode: a.mode,
      file_answer: a.file_answer,
      model: a.model,
    }),
  {
    description: 'Ask a question about a wiki; returns a cited answer. mode=fast (router) or deep (harness).',
    request_format: S.ASK_REQ,
    response_format: S.ASK_RES,
  },
);

worker.registerFunction(
  'openwiki::diagram',
  async (a) => makeDiagram(worker, { id: needId(a, 'openwiki::diagram'), kind: a.kind }),
  {
    description: 'Generate a Mermaid diagram (architecture|dataflow|deps) of a wiki.',
    request_format: S.DIAGRAM_REQ,
    response_format: S.DIAGRAM_RES,
  },
);

worker.registerFunction(
  'openwiki::export-agents-md',
  async ({ id, targets, base_url }) => exportAgentsMd(worker, { id, targets, baseUrl: base_url }),
  {
    description: 'Build the AGENTS.md/CLAUDE.md pointer block for a wiki.',
    request_format: S.EXPORT_AGENTS_REQ,
    response_format: S.EXPORT_AGENTS_RES,
  },
);

// ---------- MCP surface (DeepWiki-compatible tool names; mcp bridge exposes these) ----------

const MCP_EXPOSE = { mcp: { expose: true } };

worker.registerFunction(
  'openwiki::read-wiki-structure',
  async (a) => {
    const id = needId(a, 'openwiki::read-wiki-structure');
    const m = await store.getWiki(id);
    if (!m) throw err('openwiki/wiki_not_found', 'wiki not found');
    const pages = (await store.listPages(id)).map((x) => ({
      slug: x.slug,
      title: x.meta?.title,
      category: x.meta?.category,
    }));
    return { repo: m.repo_name, summary: m.summary, categories: m.categories || [], pages };
  },
  {
    description: "MCP: list a wiki's structure (categories + pages).",
    request_format: S.WIKI_REQ,
    response_format: S.MCP_STRUCTURE_RES,
    metadata: MCP_EXPOSE,
  },
);

worker.registerFunction(
  'openwiki::read-wiki-contents',
  async (a) => {
    const { slug } = a;
    const p = await store.getPage(needId(a, 'openwiki::read-wiki-contents'), slug);
    if (!p) throw err('openwiki/page_not_found', 'page not found');
    return { slug, ...p.meta, markdown: p.markdown };
  },
  {
    description: 'MCP: read one wiki page (markdown + metadata).',
    request_format: S.PAGE_REQ,
    response_format: S.PAGE_RES,
    metadata: MCP_EXPOSE,
  },
);

worker.registerFunction('openwiki::ask-question', async ({ id, q }) => askWiki(worker, { id, q, mode: 'fast' }), {
  description: 'MCP: ask a question about a wiki; returns a cited answer.',
  request_format: S.SEARCH_REQ,
  response_format: S.ASK_RES,
  metadata: MCP_EXPOSE,
});

// ---------- HTTP handlers ----------

const HTTP_REQ = {
  type: 'object',
  additionalProperties: true,
  properties: {
    body: { type: 'string' },
    path_params: { type: 'object', additionalProperties: { type: 'string' } },
    query_params: { type: 'object', additionalProperties: { type: 'string' } },
    headers: { type: 'object', additionalProperties: { type: 'string' } },
    method: { type: 'string' },
  },
};
const HTTP_RES = {
  type: 'object',
  additionalProperties: false,
  required: ['status_code', 'body'],
  properties: {
    status_code: { type: 'integer' },
    headers: { type: 'object', additionalProperties: { type: 'string' } },
    body: { type: 'string' },
  },
};
const HTTP_META = (description) => ({ description, request_format: HTTP_REQ, response_format: HTTP_RES });

function jsonResponse(status_code, body, extraHeaders = {}) {
  return {
    status_code,
    headers: { 'content-type': 'application/json; charset=utf-8', ...extraHeaders },
    body: JSON.stringify(body),
  };
}
function htmlResponse(status_code, html) {
  return {
    status_code,
    headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' },
    body: html,
  };
}
function parseBody(body) {
  if (body == null) return {};
  if (typeof body === 'object') return body;
  if (typeof body === 'string') {
    try {
      return JSON.parse(body);
    } catch {
      return {};
    }
  }
  return {};
}

worker.registerFunction(
  'openwiki::http::ui',
  async () => htmlResponse(200, INDEX_HTML),
  HTTP_META('HTTP: serve the OpenWiki browser UI.'),
);

worker.registerFunction(
  'openwiki::http::wikis-list',
  async () => jsonResponse(200, await store.listWikis()),
  HTTP_META('HTTP GET /openwiki/api/wikis'),
);

worker.registerFunction(
  'openwiki::http::models',
  async () => jsonResponse(200, { models: await listModels(worker), default_model: cfg.model }),
  HTTP_META('HTTP GET /openwiki/api/models'),
);

worker.registerFunction(
  'openwiki::http::wikis-create',
  async ({ body }) => {
    const payload = parseBody(body);
    if (!payload.repo_url) return jsonResponse(400, { error: 'repo_url required' });
    const { wiki_id, status } = await startWiki(payload.repo_url, payload.model, payload.ref, payload.steer);
    return jsonResponse(202, { wiki_id, status });
  },
  HTTP_META('HTTP POST /openwiki/api/wikis'),
);

worker.registerFunction(
  'openwiki::http::wiki-get',
  async ({ path_params }) => {
    const m = await store.getWiki(path_params?.id);
    return m ? jsonResponse(200, m) : jsonResponse(404, { error: 'not found' });
  },
  HTTP_META('HTTP GET /openwiki/api/wikis/:id'),
);

worker.registerFunction(
  'openwiki::http::wiki-status',
  async ({ path_params }) => {
    const s = await store.getStatus(path_params?.id);
    return jsonResponse(200, s || { phase: 'unknown', progress: 0, updated_at: now() });
  },
  HTTP_META('HTTP GET /openwiki/api/wikis/:id/status'),
);

worker.registerFunction(
  'openwiki::http::wiki-delete',
  async ({ path_params }) => {
    const id = path_params?.id;
    if (!id) return jsonResponse(400, { error: 'id required' });
    if (inflight.has(id)) return jsonResponse(409, { error: 'wiki is generating; stop it before deleting' });
    applyWikiSchedule(id, 'off'); // tear down any auto-refresh trigger for this wiki
    await store.deleteWiki(id);
    invalidateInventory(id);
    resetReadStats(id);
    return jsonResponse(200, { id, deleted: true });
  },
  HTTP_META('HTTP DELETE /openwiki/api/wikis/:id'),
);

worker.registerFunction(
  'openwiki::http::pages-list',
  async ({ path_params }) => {
    const items = await store.listPages(path_params?.id);
    return jsonResponse(
      200,
      items.map((x) => ({ slug: x.slug, ...x.meta })),
    );
  },
  HTTP_META('HTTP GET /openwiki/api/wikis/:id/pages'),
);

worker.registerFunction(
  'openwiki::http::page-get',
  async ({ path_params }) => {
    const p = await store.getPage(path_params?.id, path_params?.slug);
    return p
      ? jsonResponse(200, { slug: path_params.slug, ...p.meta, markdown: p.markdown })
      : jsonResponse(404, { error: 'not found' });
  },
  HTTP_META('HTTP GET /openwiki/api/wikis/:id/pages/:slug'),
);

worker.registerFunction(
  'openwiki::http::search',
  async ({ path_params, query_params }) => {
    const q = query_params?.q || '';
    return jsonResponse(200, await searchPages(path_params?.id, q));
  },
  HTTP_META('HTTP GET /openwiki/api/wikis/:id/search?q='),
);

worker.registerFunction(
  'openwiki::http::refresh',
  async ({ path_params }) => {
    const r = await refreshWiki(path_params?.id);
    return jsonResponse(200, r);
  },
  HTTP_META('HTTP POST /openwiki/api/wikis/:id/refresh'),
);

worker.registerFunction(
  'openwiki::http::schedule-set',
  async ({ path_params, body }) => {
    const id = path_params?.id;
    const schedule = String(parseBody(body).schedule || 'off');
    if (!SCHEDULE_PRESETS.includes(schedule) && !isRawCron(schedule))
      return jsonResponse(400, { error: 'invalid schedule' });
    const meta = await store.getWiki(id);
    if (!meta) return jsonResponse(404, { error: 'not found' });
    await store.saveWiki(id, { ...meta, refresh_schedule: schedule, updated_at: now() });
    applyWikiSchedule(id, schedule);
    return jsonResponse(200, { id, schedule, ok: true });
  },
  HTTP_META('HTTP PUT /openwiki/api/wikis/:id/schedule'),
);

// SSE live progress: stream generation events over the HTTP response channel.
worker.registerFunction(
  'openwiki::http::events',
  async (req) => {
    const wikiId = req?.path_params?.id;
    const w = req?.response;
    if (!w?.stream) return jsonResponse(501, { error: 'streaming unsupported' });
    const ctl = (o) => {
      try {
        w.sendMessage(JSON.stringify(o));
      } catch {
        /* ignore */
      }
    };
    ctl({ type: 'set_status', status_code: 200 });
    ctl({
      type: 'set_headers',
      headers: {
        'content-type': 'text/event-stream',
        'cache-control': 'no-cache',
        connection: 'keep-alive',
        'x-accel-buffering': 'no',
      },
    });

    let closed = false;
    let off = null;
    let hb = null;
    const stop = () => {
      if (closed) return;
      closed = true;
      if (hb) clearInterval(hb);
      if (off) off();
      try {
        w.close();
      } catch {
        /* ignore */
      }
    };
    // The channel socket can close under us when the worker navigates away. A
    // write then emits an ASYNC 'error' event on the Writable — a try/catch around
    // write() cannot catch it, and unhandled it crashes the whole worker. Listen
    // for it, and never write once closed.
    try {
      w.stream.on('error', stop);
    } catch {
      /* ignore */
    }
    const write = (s) => {
      if (closed) return;
      try {
        w.stream.write(s);
      } catch {
        stop();
      }
    };
    const send = (evt) => write(`data: ${JSON.stringify(evt)}\n\n`);

    const snap = await store.getStatus(wikiId);
    if (snap) send({ kind: 'status', ...snap });
    off = onProgress(wikiId, send);
    hb = setInterval(() => write(': ping\n\n'), 15_000);
    if (snap && (snap.phase === 'ready' || snap.phase === 'error')) {
      send({ kind: 'status', ...snap, final: true });
      stop();
    }
    try {
      req.request_body?.stream?.on?.('close', stop);
    } catch {
      /* ignore */
    }
    return null;
  },
  HTTP_META('HTTP GET /openwiki/api/wikis/:id/events (SSE live progress)'),
);

worker.registerFunction(
  'openwiki::http::ask',
  async ({ path_params, body }) => {
    const p = parseBody(body);
    if (!p.q) return jsonResponse(400, { error: 'q required' });
    return jsonResponse(
      200,
      await askWiki(worker, { id: path_params?.id, q: p.q, mode: p.mode, file_answer: p.file_answer }),
    );
  },
  HTTP_META('HTTP POST /openwiki/api/wikis/:id/ask'),
);

worker.registerFunction(
  'openwiki::http::diagram',
  async ({ path_params, query_params }) => {
    return jsonResponse(200, await makeDiagram(worker, { id: path_params?.id, kind: query_params?.kind }));
  },
  HTTP_META('HTTP GET /openwiki/api/wikis/:id/diagram'),
);

// ---------- HTTP triggers ----------
// api_path has NO leading slash: the engine prepends '/', and a leading slash
// double-slashes and 404s.

function bind(function_id, api_path, http_method = 'GET') {
  return worker.registerTrigger({ type: 'http', function_id, config: { api_path, http_method } });
}
bind('openwiki::http::ui', 'openwiki', 'GET');
bind('openwiki::http::ui', 'openwiki/', 'GET');
bind('openwiki::http::wikis-list', 'openwiki/api/wikis', 'GET');
bind('openwiki::http::models', 'openwiki/api/models', 'GET');
bind('openwiki::http::wikis-create', 'openwiki/api/wikis', 'POST');
bind('openwiki::http::wiki-get', 'openwiki/api/wikis/:id', 'GET');
bind('openwiki::http::wiki-delete', 'openwiki/api/wikis/:id', 'DELETE');
bind('openwiki::http::wiki-status', 'openwiki/api/wikis/:id/status', 'GET');
bind('openwiki::http::pages-list', 'openwiki/api/wikis/:id/pages', 'GET');
bind('openwiki::http::page-get', 'openwiki/api/wikis/:id/pages/:slug', 'GET');
bind('openwiki::http::search', 'openwiki/api/wikis/:id/search', 'GET');
bind('openwiki::http::refresh', 'openwiki/api/wikis/:id/refresh', 'POST');
bind('openwiki::http::schedule-set', 'openwiki/api/wikis/:id/schedule', 'PUT');
bind('openwiki::http::events', 'openwiki/api/wikis/:id/events', 'GET');
bind('openwiki::http::ask', 'openwiki/api/wikis/:id/ask', 'POST');
bind('openwiki::http::diagram', 'openwiki/api/wikis/:id/diagram', 'GET');

// ---------- Scheduled refresh (per-wiki cron triggers) ----------
// Each wiki carries its own auto-refresh cadence, set from the UI, never
// hardcoded. openwiki::set-schedule registers a per-wiki cron trigger; every
// such trigger fires openwiki::cron::refresh-due, which refreshes the wikis whose
// interval has elapsed. Trigger handles live in wikiTriggers so a schedule change
// or delete can unregister the old one; on boot they are re-registered from the
// stored schedules.
const wikiTriggers = new Map(); // wikiId -> unregister fn

const REFRESH_INTERVALS = {
  '3h': { cron: '0 0 */3 * * *', ms: 3 * 3600e3 },
  '6h': { cron: '0 0 */6 * * *', ms: 6 * 3600e3 },
  '12h': { cron: '0 0 */12 * * *', ms: 12 * 3600e3 },
  daily: { cron: '0 0 3 * * *', ms: 24 * 3600e3 },
  weekly: { cron: '0 0 3 * * 1', ms: 7 * 24 * 3600e3 },
};
const SCHEDULE_PRESETS = ['off', ...Object.keys(REFRESH_INTERVALS)];
const isRawCron = (s) => /^(\S+\s+){4,5}\S+$/.test(String(s || ''));

// Register (or replace) one wiki's cron trigger. 'off' just unregisters. A raw
// 5-6 field cron is accepted for power users; its due window is the trigger
// cadence itself (refresh whenever it fires).
function applyWikiSchedule(wikiId, schedule) {
  const prev = wikiTriggers.get(wikiId);
  if (prev) {
    try {
      prev();
    } catch {
      /* already gone */
    }
    wikiTriggers.delete(wikiId);
  }
  if (!schedule || schedule === 'off') return;
  const cron = REFRESH_INTERVALS[schedule] ? REFRESH_INTERVALS[schedule].cron : schedule;
  try {
    const { unregister } = worker.registerTrigger({
      type: 'cron',
      function_id: 'openwiki::cron::refresh-due',
      config: { schedule: cron },
      metadata: { wiki_id: wikiId },
    });
    wikiTriggers.set(wikiId, unregister);
  } catch (e) {
    console.warn('[openwiki] failed to schedule', wikiId, e?.message || e);
  }
}

// A scheduled cron fired: refresh every wiki whose interval has elapsed. All the
// per-wiki triggers point here; the due check keeps it correct regardless of
// which one fired, and idempotent (content-hash gate + inflight guard).
worker.registerFunction(
  'openwiki::cron::refresh-due',
  async () => {
    const wikis = await store.listWikis();
    let refreshed = 0;
    for (const w of wikis) {
      const sched = w.refresh_schedule;
      if (!sched || sched === 'off' || inflight.has(w.id)) continue;
      const dueMs = REFRESH_INTERVALS[sched]?.ms || 0; // raw cron: due whenever it fires
      const last = w.last_refresh_at ? Date.parse(w.last_refresh_at) : 0;
      if (Date.now() - last < dueMs - 60_000) continue; // 1-minute slack against cron drift
      try {
        await refreshWiki(w.id);
        refreshed += 1;
      } catch (e) {
        console.error('[openwiki] scheduled refresh failed', w.id, e?.message);
      }
    }
    return { refreshed };
  },
  {
    description: 'Cron: refresh every wiki whose auto-refresh interval has elapsed.',
    request_format: { type: 'object', additionalProperties: true, properties: {} },
    response_format: {
      type: 'object',
      additionalProperties: false,
      required: ['refreshed'],
      properties: { refreshed: { type: 'integer' } },
    },
  },
);

// Set (or clear) a wiki's auto-refresh cadence and (re)register its cron trigger.
worker.registerFunction(
  'openwiki::set-schedule',
  async (a) => {
    const id = needId(a, 'openwiki::set-schedule');
    const schedule = String(a.schedule || 'off');
    if (!SCHEDULE_PRESETS.includes(schedule) && !isRawCron(schedule)) {
      throw err(
        'openwiki/bad_request',
        `schedule must be one of ${SCHEDULE_PRESETS.join(', ')} or a 5-6 field cron string`,
      );
    }
    const meta = await store.getWiki(id);
    if (!meta) throw err('openwiki/wiki_not_found', 'wiki not found');
    await store.saveWiki(id, { ...meta, refresh_schedule: schedule, updated_at: now() });
    applyWikiSchedule(id, schedule);
    return { id, schedule, ok: true };
  },
  {
    description:
      'Set a wiki auto-refresh cadence: off | 3h | 6h | 12h | daily | weekly | a cron string. Params: { id, schedule }.',
    request_format: S.SET_SCHEDULE_REQ,
    response_format: S.SET_SCHEDULE_RES,
  },
);

// Real-time page collection: subscribe to the harness's emitted turn-completed
// events and route each to the active generation (turnbus). One broad binding
// (no session filter) sees every turn; unowned roots are a cheap map miss. This
// drives the orchestrator's live per-page progress instead of polling.
worker.registerFunction(
  'openwiki::on-turn-completed',
  async (evt) => {
    try {
      turnbus.deliver(evt?.payload || evt);
    } catch (e) {
      console.warn('[openwiki] turn-event route failed', e?.message || e);
    }
    return null;
  },
  {
    description: 'Internal: routes harness::turn-completed events to the active generation.',
    request_format: { type: 'object', additionalProperties: true, properties: {} },
    response_format: { type: 'null' },
  },
);

// turn-started drives the "spawned page-writer" line in the live feed.
worker.registerFunction(
  'openwiki::on-turn-started',
  async (evt) => {
    try {
      turnbus.deliverStarted(evt?.payload || evt);
    } catch (e) {
      console.warn('[openwiki] turn-started route failed', e?.message || e);
    }
    return null;
  },
  {
    description: 'Internal: routes harness::turn-started events (sub-agent spawns) to the active generation.',
    request_format: { type: 'object', additionalProperties: true, properties: {} },
    response_format: { type: 'null' },
  },
);

Promise.resolve()
  .then(() =>
    worker.registerTrigger({ type: 'harness::turn-completed', function_id: 'openwiki::on-turn-completed', config: {} }),
  )
  .then(() =>
    worker.registerTrigger({ type: 'harness::turn-started', function_id: 'openwiki::on-turn-started', config: {} }),
  )
  .then(() => {
    console.log('[openwiki] subscribed to harness turn events (real-time collection + spawn feed)');
  })
  .catch((e) => console.warn('[openwiki] turn-event subscribe failed; using synchronous fallback:', e?.message || e));

// Configuration: register the schema, load the stored value, and hot-reload on
// change. Runs off the boot path so it never delays function registration.
configuration
  .registerConfig(worker)
  .then(() => configuration.fetchConfig(worker))
  .then((c) => {
    cfg = c;
  })
  .catch((e) => console.warn('[openwiki] config register failed; using defaults', e?.message || e));
configuration.bindConfigTrigger(worker, async () => {
  cfg = await configuration.fetchConfig(worker);
});

// Reap generations orphaned by a restart. On a fresh boot nothing is in-flight,
// so any wiki still flagged generating was interrupted mid-run. Clear the flag,
// reconcile page_count to what actually landed on disk, and mark the status so
// the UI stops showing a forever-running progress panel. The partial pages that
// were written stay browsable. Runs off the boot path.
(async () => {
  try {
    const wikis = await store.listWikis();
    for (const w of wikis) {
      if (!w.generating || inflight.has(w.id)) continue;
      const pages = await store.listPages(w.id).catch(() => []);
      await store.saveWiki(w.id, { ...w, generating: false, page_count: pages.length, updated_at: now() });
      await store.updateStatus(w.id, {
        phase: pages.length ? 'ready' : 'error',
        progress: pages.length ? 1 : 0,
        message: `Interrupted by a restart with ${pages.length} page(s). Refresh to complete.`,
        updated_at: now(),
      });
      await store.appendLog(w.id, `Reaped orphaned generation (${pages.length} pages on disk).`);
    }
    // Cron triggers are in-memory registrations, so re-arm each wiki's stored
    // auto-refresh cadence after a restart.
    for (const w of wikis) {
      if (w.refresh_schedule && w.refresh_schedule !== 'off') applyWikiSchedule(w.id, w.refresh_schedule);
    }
  } catch (e) {
    console.warn('[openwiki] reaper failed', e?.message || e);
  }
})();

console.log('[openwiki] worker ready — model default =', cfg.model, 'iii url =', III_URL);

process.on('SIGTERM', async () => {
  try {
    await worker.shutdown();
  } catch {}
  process.exit(0);
});
process.on('SIGINT', async () => {
  try {
    await worker.shutdown();
  } catch {}
  process.exit(0);
});
