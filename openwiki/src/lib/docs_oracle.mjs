// Official-docs oracle. A project's own documentation index is the strongest
// information-architecture signal (vercel-labs/openwiki's biggest quality
// lever). We look for an llms.txt at the doc sites the README links to, and
// fall back to the repo's docs/ tree, then derive a nav hint + page budget the
// planner uses to build breadth comparable to the official docs.
import fs from 'node:fs/promises';
import path from 'node:path';

// Parse an llms.txt: markdown with section headings and "- [title](url)" links.
export function parseLlmsTxt(text) {
  const sections = [];
  let current = null;
  const links = [];
  for (const raw of String(text || '').split(/\r?\n/)) {
    const h = raw.match(/^#{1,3}\s+(.+)$/);
    if (h) {
      current = { title: h[1].replace(/[#*`]/g, '').trim(), links: [] };
      sections.push(current);
      continue;
    }
    const l = raw.match(/^\s*[-*]\s+\[([^\]]+)\]\(([^)\s]+)/);
    if (l) {
      const item = { title: l[1].trim(), url: l[2] };
      links.push(item);
      if (current) current.links.push(item);
    }
  }
  return { sections: sections.filter((s) => s.links.length), links };
}

export function candidateOrigins(_repoUrl, readme) {
  const origins = new Set();
  for (const u of String(readme || '').match(/https?:\/\/[^\s)\]]+/g) || []) {
    try {
      const o = new URL(u);
      if (/github\.com|githubusercontent|shields\.io|npmjs\.com|badge|codecov|circleci/i.test(o.hostname)) continue;
      origins.add(o.origin);
    } catch {
      /* skip bad url */
    }
  }
  return [...origins].slice(0, 4);
}

async function webFetch(worker, url, timeoutMs = 15_000) {
  try {
    const res = await worker.trigger({
      function_id: 'web::fetch',
      payload: { url, format: 'markdown' },
      timeoutMs,
    });
    const status = res?.status ?? res?.status_code ?? 200;
    const body = res?.content ?? res?.body ?? res?.markdown ?? res?.text ?? '';
    if (status >= 400 || !body) return null;
    return String(body);
  } catch {
    return null;
  }
}

export async function fetchDocsIndex(worker, { repoUrl, readme, repoDir }) {
  // Overall budget across every probe: up to 8 sequential fetches against
  // origins that may all be black holes must not stall planning for minutes.
  const deadline = Date.now() + 45_000;
  // 1) llms.txt at README-referenced documentation origins.
  outer: for (const origin of candidateOrigins(repoUrl, readme)) {
    for (const p of ['/llms.txt', '/llms-full.txt']) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) break outer;
      const txt = await webFetch(worker, origin + p, Math.min(15_000, remaining));
      if (txt && /\]\(https?:/.test(txt)) {
        const parsed = parseLlmsTxt(txt);
        if (parsed.links.length >= 5)
          return { source: origin + p, linkCount: parsed.links.length, sections: parsed.sections };
      }
    }
  }
  // 2) the repo's own docs/ tree.
  try {
    const docsDir = path.join(repoDir, 'docs');
    const st = await fs.stat(docsDir).catch(() => null);
    if (st?.isDirectory()) {
      const files = [];
      const walk = async (d, rel = '') => {
        if (files.length > 400) return;
        for (const e of await fs.readdir(d, { withFileTypes: true })) {
          if (files.length > 400) return;
          if (e.isDirectory()) await walk(path.join(d, e.name), `${rel + e.name}/`);
          else if (/\.mdx?$/i.test(e.name)) files.push(rel + e.name);
        }
      };
      await walk(docsDir);
      if (files.length >= 5) {
        const groups = {};
        for (const f of files) {
          const g = f.includes('/') ? f.split('/')[0] : 'docs';
          (groups[g] = groups[g] || []).push(f);
        }
        const sections = Object.entries(groups).map(([title, fs2]) => ({
          title,
          links: fs2.map((f) => ({ title: f, url: f })),
        }));
        return { source: 'docs/', linkCount: files.length, sections };
      }
    }
  } catch {
    /* no docs tree */
  }
  return null;
}

// Map a discovered docs index to a page budget (vercel's adaptive ladder).
export function docsBudget(linkCount) {
  if (linkCount >= 260) return 48;
  if (linkCount >= 160) return 40;
  if (linkCount >= 90) return 34;
  if (linkCount >= 40) return 26;
  return 18;
}

// A compact hint injected into the plan prompt.
export function docsHint(docsIndex) {
  if (!docsIndex) return '';
  const titles = docsIndex.sections
    .map((s) => s.title)
    .filter(Boolean)
    .slice(0, 12);
  return (
    `\n\nOfficial documentation index discovered (${docsIndex.linkCount} topics, source: ${docsIndex.source}).\n` +
    'Treat this as the STRONGEST information-architecture signal. Preserve its major sections as top-level nav folders and give the wiki comparable breadth.\n' +
    (titles.length ? `Major sections to mirror: ${titles.join(', ')}.\n` : '') +
    `Target about ${docsBudget(docsIndex.linkCount)} pages.`
  );
}
