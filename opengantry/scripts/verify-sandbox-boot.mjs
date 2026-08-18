#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const root = path.resolve(import.meta.dirname, '..');
const bundle = path.join(root, 'sandbox.mjs');
if (!fs.existsSync(bundle)) {
  console.error('verify:sandbox: run pnpm run build:bundle first');
  process.exit(1);
}

const ALLOWED_BOOT_FAILURE =
  /ECONNREFUSED|connect ECONNREFUSED|WebSocket|failed to connect|connection refused/i;

const bundleUrl = pathToFileURL(bundle).href;
const result = spawnSync(
  process.execPath,
  [
    '--input-type=module',
    '-e',
    `const url = ${JSON.stringify(bundleUrl)};
process.env.III_URL = 'ws://127.0.0.1:1';
try {
  await import(url);
} catch (e) {
  const msg = String(e?.message ?? e);
  if (${ALLOWED_BOOT_FAILURE}.test(msg)) {
    process.exit(0);
  }
  console.error('verify:sandbox: unexpected boot error:', msg);
  process.exit(1);
}
process.exit(0);`,
  ],
  {
    env: { ...process.env, III_URL: 'ws://127.0.0.1:1' },
    timeout: 15_000,
    stdio: 'inherit',
  },
);

if (result.error?.code === 'ETIMEDOUT') {
  console.error('verify:sandbox: bundle import timed out');
  process.exit(1);
}

process.exit(result.status ?? 1);
