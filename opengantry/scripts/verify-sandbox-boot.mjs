#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

/** Pinned sandbox runtime — docker.io/iiidev/node:latest @ 2026-03-23 */
const SANDBOX_IMAGE =
  'docker.io/iiidev/node@sha256:a9b6a5354afa648f4a07ad95c03163a198668af0df314410dc557b9786fb47cb';

const root = path.resolve(import.meta.dirname, '..');
const bundle = path.join(root, 'sandbox.mjs');
if (!fs.existsSync(bundle)) {
  console.error('verify:sandbox: run pnpm run build:bundle first');
  process.exit(1);
}

const result = spawnSync(
  'docker',
  [
    'run',
    '--rm',
    '-v',
    `${bundle}:/w/index.mjs:ro`,
    '-w',
    '/w',
    SANDBOX_IMAGE,
    'node',
    '--input-type=module',
    '-e',
    'await import("/w/index.mjs")',
  ],
  { stdio: 'inherit' },
);

process.exit(result.status ?? 1);
