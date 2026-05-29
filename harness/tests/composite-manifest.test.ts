import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..');
const srcDir = join(repoRoot, 'src');
const indexEntry = join(srcDir, 'index.ts');
const tsxBin = join(repoRoot, 'node_modules', '.bin', 'tsx');

const pkg = JSON.parse(readFileSync(join(repoRoot, 'package.json'), 'utf8')) as {
  scripts: Record<string, string>;
  bin: Record<string, string>;
};

/**
 * Every folder under `src/` that ships a `main.ts` is a worker meant to run
 * standalone. The composite entry-point (`src/index.ts`, what `dev:all` and
 * `start:all` run) must register every one of them — otherwise a worker
 * silently never starts in the all-in-one process. Regression guard for #202,
 * where the `web` worker was added but never wired into `index.ts`.
 */
function runnableWorkerFolders(): string[] {
  return readdirSync(srcDir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && existsSync(join(srcDir, e.name, 'main.ts')))
    .map((e) => e.name)
    .sort();
}

function compositeManifestNames(): string[] {
  const out = execFileSync(tsxBin, [indexEntry, '--manifest'], { encoding: 'utf8' });
  const manifest = JSON.parse(out) as Array<{ name: string }>;
  return manifest.map((w) => w.name).sort();
}

describe('composite entry-point worker registration', () => {
  it('registers every runnable worker folder in the WORKERS array', () => {
    expect(compositeManifestNames()).toEqual(runnableWorkerFolders());
  });

  it('exposes a dev:<name> script for every runnable worker', () => {
    const missing = runnableWorkerFolders().filter((name) => !pkg.scripts[`dev:${name}`]);
    expect(missing).toEqual([]);
  });

  it('exposes an iii-<name> bin for every runnable worker', () => {
    const missing = runnableWorkerFolders().filter((name) => !pkg.bin[`iii-${name}`]);
    expect(missing).toEqual([]);
  });
});
