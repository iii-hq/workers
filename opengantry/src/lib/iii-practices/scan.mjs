import fs from 'node:fs';
import path from 'node:path';
import { findOrphanSourceDirs } from './scan-workers.mjs';
import { checkAsyncBoundaries } from './check-async-boundaries.mjs';
import { checkPayloadContracts } from './check-payload-contracts.mjs';
import { checkDurableState } from './check-durable-state.mjs';
import { checkWorkerIsolation } from './check-worker-isolation.mjs';
import { checkWorkerManifest } from './check-manifest.mjs';

export async function scanWorkersTree(scanRoot, options = {}) {
  const findings = [];
  findings.push(...findOrphanSourceDirs(scanRoot));
  findings.push(...checkWorkerManifest(scanRoot));
  findings.push(...(await checkAsyncBoundaries(scanRoot, options)));
  findings.push(...(await checkPayloadContracts(scanRoot)));
  const durable = await checkDurableState(scanRoot);
  findings.push(...durable.findings);
  findings.push(...(await checkWorkerIsolation(scanRoot)));
  return { findings, logs: durable.logs };
}

export function localWorkersRoot(repoRoot) {
  return path.join(repoRoot, 'workers');
}

/**
 * Scan the adopter's local workers/ tree. Missing workers/ is a no-op (not a fail).
 * Registry-installed workers outside this tree are not scanned.
 */
export async function scanLocalWorkers(repoRoot, options = {}) {
  const scanRoot = options.scanRoot ?? localWorkersRoot(repoRoot);
  if (!fs.existsSync(scanRoot)) {
    return { findings: [], logs: [] };
  }
  try {
    fs.readdirSync(scanRoot);
  } catch (e) {
    return {
      findings: [
        {
          rule_id: 'scan/unreadable',
          file: 'workers',
          line: 1,
          message: `local workers/ exists but is unreadable: ${e.message}`,
        },
      ],
      logs: [],
    };
  }
  try {
    return await scanWorkersTree(scanRoot, options);
  } catch (e) {
    return {
      findings: [
        {
          rule_id: 'scan/unreadable',
          file: 'workers',
          line: 1,
          message: `local workers/ scan aborted: ${e.message}`,
        },
      ],
      logs: [],
    };
  }
}

export function practicesFailedPayload(findings) {
  return {
    status: 'failed',
    phase: 'iii-practices',
    message: 'iii-practices scan failed on local workers/',
    error_code: 'GXT_GATE_FAILED',
    exit_code: 1,
    findings: findings.map((f) => ({
      failed_gate: 'iii-practices',
      resolution_hint: `[${f.rule_id}] ${f.file}:${f.line} ${f.message}`,
    })),
    fix_hints: findings.map((f) => `[${f.rule_id}] ${f.file}:${f.line} ${f.message}`),
    next_actions: ['Fix local workers/ to match iii worker contracts, then re-run gantry::verify'],
  };
}
