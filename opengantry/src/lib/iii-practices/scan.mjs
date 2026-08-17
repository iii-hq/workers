import path from 'node:path';

import { preflightDeps } from './scan-workers.mjs';
import { checkWorkerManifest } from './check-manifest.mjs';
import { checkPayloadSchemaFiles } from './check-payload-contracts.mjs';
import { findOrphanSourceDirs } from './scan-workers.mjs';
import { runSourceRules } from './run-source-rules.mjs';
import { loadHttpConnectorAllowlist } from './allowlist.mjs';

const INFRA_RULE_IDS = new Set(['scan/deps-missing', 'scan/deps-unreadable', 'scan/parse']);

export function practicesFailedPayload(findings) {
  return {
    status: 'failed',
    findings: findings.map((f) => ({
      failed_gate: INFRA_RULE_IDS.has(f.rule_id) ? 'infra' : 'architecture',
      resolution_hint: `${f.rule_id}: ${f.message} (${f.file}:${f.line ?? 1})`,
    })),
  };
}

/**
 * Unified practices scan: preflight, AST rules (single pass), manifest, schemas, orphans.
 * @param {string} scanRoot
 * @param {{ allowlistRoot?: string, httpAllowlist?: Set<string> }} [options]
 */
export async function runPracticesScan(scanRoot, options = {}) {
  const logs = [];
  try {
    preflightDeps();
  } catch (e) {
    return {
      findings: [
        {
          rule_id: 'scan/deps-missing',
          file: '.',
          line: 1,
          message: e instanceof Error ? e.message : String(e),
        },
      ],
      logs,
    };
  }

  const allowlistRoot = options.allowlistRoot ?? scanRoot;
  const { workers: httpAllowlist } = loadHttpConnectorAllowlist(allowlistRoot);
  const mergedAllowlist = options.httpAllowlist ?? httpAllowlist;

  const source = await runSourceRules(scanRoot, { httpAllowlist: mergedAllowlist });
  logs.push(...(source.logs ?? []));
  const findings = [...(source.findings ?? [])];

  findings.push(...checkWorkerManifest(scanRoot));
  checkPayloadSchemaFiles(scanRoot, findings);
  findings.push(...findOrphanSourceDirs(scanRoot));

  return { findings, logs };
}

/** @deprecated use runPracticesScan */
export async function scanLocalWorkers(scanRoot, options = {}) {
  return runPracticesScan(scanRoot, {
    allowlistRoot: options.allowlistRoot,
    httpAllowlist: options.httpAllowlist,
  });
}

/** @deprecated alias for self-tests */
export const scanWorkersTree = scanLocalWorkers;

export { runSourceRules };
