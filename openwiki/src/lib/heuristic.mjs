// Heuristic fallback for OpenWiki: produces a serviceable, source-grounded wiki
// without an LLM. Used when router::complete is unavailable or errors.
import { readFile } from 'node:fs/promises';
import path from 'node:path';

function titleCase(s) {
  return String(s || '')
    .replace(/[-_]/g, ' ')
    .replace(/\b\w/g, (c) => c.toUpperCase())
    .trim();
}

function slugify(s) {
  return (
    String(s || '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 60) || 'page'
  );
}

function firstParagraph(text, cap = 400) {
  if (!text) return '';
  const lines = text.split(/\r?\n/);
  const out = [];
  let started = false;
  for (const raw of lines) {
    const l = raw.trim();
    if (!started) {
      if (!l) continue;
      if (l.startsWith('#')) continue;
      if (l.startsWith('!') || l.startsWith('[![')) continue;
      started = true;
      out.push(l);
    } else {
      if (!l) break;
      if (l.startsWith('#')) break;
      out.push(l);
    }
    if (out.join(' ').length > cap) break;
  }
  return out.join(' ').slice(0, cap);
}

async function readIfExists(dir, rel, cap = 12_000) {
  try {
    const buf = await readFile(path.join(dir, rel), 'utf8');
    return buf.length > cap ? buf.slice(0, cap) : buf;
  } catch {
    return null;
  }
}

function topLevelDirs(inventory) {
  const dirs = new Map();
  for (const e of inventory) {
    if ((e.priority ?? 0) <= 0) continue;
    const first = e.relPath.split('/')[0];
    if (!first || first.includes('.')) continue;
    if (!dirs.has(first)) dirs.set(first, []);
    dirs.get(first).push(e);
  }
  return [...dirs.entries()].filter(([, files]) => files.length >= 2).sort((a, b) => b[1].length - a[1].length);
}

function packageInfo(pkgText) {
  if (!pkgText) return null;
  try {
    const j = JSON.parse(pkgText);
    return {
      name: j.name || null,
      version: j.version || null,
      description: j.description || null,
      scripts: j.scripts || {},
      dependencies: Object.keys(j.dependencies || {}),
      devDependencies: Object.keys(j.devDependencies || {}),
    };
  } catch {
    return null;
  }
}

export async function planWikiHeuristic({ inventory, repoName, repoDir }) {
  const readme = (await readIfExists(repoDir, 'README.md')) || (await readIfExists(repoDir, 'README.mdx')) || '';
  const pkg = packageInfo((await readIfExists(repoDir, 'package.json')) || '');
  const summary =
    firstParagraph(readme, 400) ||
    (pkg?.description ? String(pkg.description) : '') ||
    `Source-grounded wiki for ${repoName}.`;

  const categories = [
    { id: 'overview', title: 'Overview', description: 'What this repository is and how it fits together.' },
    { id: 'architecture', title: 'Architecture', description: 'How the source tree is organized.' },
    { id: 'reference', title: 'Reference', description: 'File-by-file inventory and configuration.' },
    { id: 'docs', title: 'Docs', description: 'Repository documentation, verbatim.' },
  ];

  const outline = [];
  const invPaths = new Set(inventory.map((e) => e.relPath));

  // 1) Overview page (always)
  const overviewSources = [];
  if (invPaths.has('README.md')) overviewSources.push('README.md');
  else if (invPaths.has('README.mdx')) overviewSources.push('README.mdx');
  if (invPaths.has('package.json')) overviewSources.push('package.json');
  outline.push({
    slug: 'overview',
    title: 'Overview',
    category: 'overview',
    source_paths: overviewSources.length ? overviewSources : [inventory[0]?.relPath].filter(Boolean),
    brief: `High-level summary of ${repoName}.`,
  });

  // 2) Getting Started (if package.json or install docs exist)
  if (invPaths.has('package.json')) {
    outline.push({
      slug: 'getting-started',
      title: 'Getting Started',
      category: 'overview',
      source_paths: ['package.json', ...['README.md', 'README.mdx'].filter((p) => invPaths.has(p))],
      brief: 'Install, scripts, and quick start.',
    });
  }

  // 3) Architecture pages — one per top-level dir with code
  const dirs = topLevelDirs(inventory);
  for (const [dir, files] of dirs.slice(0, 5)) {
    const top = files
      .sort((a, b) => (b.priority ?? 0) - (a.priority ?? 0))
      .slice(0, 12)
      .map((f) => f.relPath);
    outline.push({
      slug: `dir-${slugify(dir)}`,
      title: `${titleCase(dir)} Directory`,
      category: 'architecture',
      source_paths: top,
      brief: `Contents and role of the \`${dir}/\` directory.`,
    });
  }

  // 4) One page per doc file (up to 6)
  const docs = inventory.filter((e) => e.isDoc && e.relPath !== 'README.md' && e.relPath !== 'README.mdx').slice(0, 6);
  for (const d of docs) {
    outline.push({
      slug: `doc-${slugify(d.relPath)}`,
      title: titleCase(path.basename(d.relPath).replace(/\.[^.]+$/, '')),
      category: 'docs',
      source_paths: [d.relPath],
      brief: `Repository documentation: ${d.relPath}.`,
    });
  }

  // 5) File reference page — full high-priority inventory
  const refTop = inventory.slice(0, 30).map((e) => e.relPath);
  outline.push({
    slug: 'file-reference',
    title: 'File Reference',
    category: 'reference',
    source_paths: refTop,
    brief: 'Curated inventory of the most relevant files in the repository.',
  });

  return { summary, categories, outline };
}

export async function generatePageHeuristic({
  outlineItem,
  sourceReads,
  allSlugs,
  allTitles,
  categories,
  repoName,
  repoUrl,
}) {
  const title = outlineItem.title;
  const category = categories.find((c) => c.id === outlineItem.category);
  const catTitle = category ? category.title : outlineItem.category;

  const lines = [];
  lines.push(`# ${title}`);
  lines.push('');
  lines.push(`_Category: **${catTitle}**  ·  Repo: [${repoName}](${repoUrl})_`);
  lines.push('');
  lines.push('## Overview');
  lines.push('');
  lines.push(outlineItem.brief || `Notes on ${title.toLowerCase()} for ${repoName}.`);
  lines.push('');

  // Try to derive a short description from the first source's leading comment/paragraph
  const first = sourceReads[0];
  if (first) {
    const preview = extractSummary(first.content, first.path);
    if (preview) {
      lines.push(preview);
      lines.push('');
    }
  }

  // Key files section
  lines.push('## Key files');
  lines.push('');
  for (const sr of sourceReads) {
    const short = oneLine(sr.content);
    lines.push(`- \`${sr.path}\`${short ? ` — ${short}` : ''}`);
  }
  lines.push('');

  // Excerpts
  const excerpts = sourceReads
    .filter((sr) => !isBinaryLikely(sr.path) && sr.content && sr.content.length > 0)
    .slice(0, 4);
  if (excerpts.length) {
    lines.push('## Excerpts');
    lines.push('');
    for (const sr of excerpts) {
      const ext = extFromPath(sr.path);
      const body = truncateLines(sr.content, 40);
      lines.push(`### \`${sr.path}\``);
      lines.push('');
      lines.push(`\`\`\`${ext}`);
      lines.push(body);
      lines.push('```');
      lines.push('');
    }
  }

  // Related pages
  const related = [];
  for (let i = 0; i < allSlugs.length; i++) {
    if (allSlugs[i] === outlineItem.slug) continue;
    related.push(`- [${allTitles[i]}](./${allSlugs[i]}.md)`);
    if (related.length >= 6) break;
  }
  if (related.length) {
    lines.push('## Related pages');
    lines.push('');
    lines.push(...related);
    lines.push('');
  }

  // Sources
  lines.push('## Sources');
  lines.push('');
  for (const p of outlineItem.source_paths) {
    lines.push(`- \`${p}\``);
  }
  lines.push('');
  lines.push(`_Generated heuristically (no LLM) at ${new Date().toISOString()}._`);

  const markdown = lines.join('\n');
  const frontmatter = {
    title,
    slug: outlineItem.slug,
    category: outlineItem.category,
    source_paths: outlineItem.source_paths,
    last_updated: new Date().toISOString(),
    confidence: 'medium',
    status: 'current',
    generator: 'heuristic',
  };
  return { markdown, frontmatter };
}

function extFromPath(p) {
  const m = String(p ?? '').match(/\.([A-Za-z0-9]+)$/);
  return m ? m[1].toLowerCase() : 'text';
}

function isBinaryLikely(p) {
  return /\.(png|jpg|jpeg|gif|webp|ico|pdf|zip|tar|gz|bin|wasm|woff2?|ttf|otf|mp[34]|mov|svg)$/i.test(p);
}

function oneLine(content) {
  if (!content) return '';
  const lines = String(content).split(/\r?\n/);
  for (const raw of lines) {
    const l = raw.replace(/^[\s#*/-]+/, '').trim();
    if (l && !l.startsWith('```')) return l.slice(0, 120);
  }
  return '';
}

function extractSummary(content, filePath) {
  if (!content) return '';
  // Prefer a top-of-file block comment / docstring
  const m1 = content.match(/^(?:\s*\/\*\*?([\s\S]*?)\*\/)/);
  if (m1) {
    const body = m1[1]
      .split(/\r?\n/)
      .map((l) => l.replace(/^\s*\*?\s?/, '').trim())
      .filter(Boolean)
      .join(' ')
      .trim();
    if (body.length > 20) return body.slice(0, 400);
  }
  // Python docstring
  const m2 = content.match(/^\s*"""([\s\S]*?)"""/);
  if (m2 && m2[1].trim().length > 20) return m2[1].trim().slice(0, 400);
  // Markdown: first paragraph
  if (/\.mdx?$/i.test(filePath)) return firstParagraph(content, 400);
  // Fallback: first meaningful line as a summary
  return '';
}

function truncateLines(text, n) {
  const arr = String(text).split(/\r?\n/);
  if (arr.length <= n) return text;
  return `${arr.slice(0, n).join('\n')}\n// ...[truncated]`;
}
