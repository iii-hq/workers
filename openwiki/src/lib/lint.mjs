// Lint pass: validate that a wiki's pages are still grounded. Checks every
// citation resolves to a real file and a line range inside it, and flags thin
// pages. Runs after generation/refresh and on the nightly cron. Orphan and
// missing-cross-reference checks are a later addition.
import fs from 'node:fs/promises';
import * as store from './store.mjs';
import { readSourceFile } from './inventory.mjs';

const THIN_CHARS = 200;

export async function lintWiki(wikiId) {
  const meta = await store.getWiki(wikiId);
  if (!meta) {
    const e = new Error('wiki not found');
    e.code = 'openwiki/wiki_not_found';
    throw e;
  }
  const dir = store.repoDir(wikiId);
  const haveClone = !!(await fs.stat(dir).catch(() => null));
  const pages = await store.listPages(wikiId);
  const issues = [];
  let checked = 0;

  for (const { slug, meta: pm } of pages) {
    checked += 1;

    if (haveClone) {
      for (const c of pm?.citations || []) {
        if (!c?.path) continue;
        let content;
        try {
          ({ content } = await readSourceFile(dir, c.path, 200_000));
        } catch {
          issues.push({ slug, kind: 'broken-citation', detail: `missing file ${c.path}` });
          continue;
        }
        if (c.start_line || c.end_line) {
          const total = content.split(/\r?\n/).length;
          if (c.start_line && c.start_line > total) {
            issues.push({ slug, kind: 'broken-citation', detail: `${c.path}:${c.start_line} beyond ${total} lines` });
          } else if (c.end_line && c.end_line > total) {
            issues.push({ slug, kind: 'broken-citation', detail: `${c.path}:${c.end_line} beyond ${total} lines` });
          }
        }
      }
    }

    const page = await store.getPage(wikiId, slug);
    if (page && String(page.markdown || '').trim().length < THIN_CHARS) {
      issues.push({ slug, kind: 'thin', detail: `page body under ${THIN_CHARS} chars` });
    }
  }

  return { checked, issues };
}
