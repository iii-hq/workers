#!/usr/bin/env node
/**
 * Copy non-TS assets (JSON seeds, prompts, iii.worker.yaml manifests)
 * into dist/ alongside the compiled files. Mirrors the Rust crates'
 * `include_str!` for embedded data.
 */

import { cp, mkdir, readdir } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const srcDir = join(root, 'src');
const distDir = join(root, 'dist');

const exts = new Set(['.json', '.txt', '.md', '.yaml', '.yml']);

async function walk(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const out = [];
  for (const e of entries) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      out.push(...(await walk(p)));
    } else if (e.isFile()) {
      const dot = e.name.lastIndexOf('.');
      const ext = dot >= 0 ? e.name.slice(dot) : '';
      if (exts.has(ext)) out.push(p);
    }
  }
  return out;
}

const files = await walk(srcDir);
for (const f of files) {
  const rel = relative(srcDir, f);
  const target = join(distDir, rel);
  await mkdir(dirname(target), { recursive: true });
  await cp(f, target);
}
console.log(`copied ${files.length} asset(s) to dist/`);
