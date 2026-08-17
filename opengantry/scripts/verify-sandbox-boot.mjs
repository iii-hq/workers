#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

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
    'docker.io/iiidev/node:latest',
    'node',
    '--input-type=module',
    '-e',
    'await import("/w/index.mjs")',
  ],
  { stdio: 'inherit' },
);

process.exit(result.status ?? 1);
