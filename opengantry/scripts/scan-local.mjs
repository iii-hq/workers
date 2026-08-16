#!/usr/bin/env node
/**
 * Host-side scan of <cwd>/workers (or argv[2] repo root).
 * Used as the default bootstrap mission gate_command.
 */
import path from 'node:path';
import { preflightDeps } from '../src/lib/iii-practices/scan-workers.mjs';
import { scanLocalWorkers } from '../src/lib/iii-practices/scan.mjs';
import {
  resolveRepoRoot,
  loadHttpConnectorAllowlist,
} from '../src/lib/iii-practices/allowlist.mjs';

const repoRoot = process.argv[2] ? path.resolve(process.argv[2]) : process.cwd();

try {
  preflightDeps();
} catch (e) {
  console.error(e.message);
  process.exit(2);
}

let httpAllowlist;
try {
  ({ workers: httpAllowlist } = loadHttpConnectorAllowlist(resolveRepoRoot()));
} catch (e) {
  console.error(
    `FATAL: EXIT 2 — scanner could not run; this is NOT an architecture violation. ${e.message}`,
  );
  process.exit(2);
}
const { findings, logs } = await scanLocalWorkers(repoRoot, { httpAllowlist });
for (const line of logs) console.log(line);
if (findings.length) {
  console.error('iii-practices: local workers/ scan failed:');
  for (const f of findings) {
    console.error(`  [${f.rule_id}] ${f.file}:${f.line} ${f.message}`);
  }
  process.exit(1);
}
console.log('[iii-practices: local workers scan OK]');
