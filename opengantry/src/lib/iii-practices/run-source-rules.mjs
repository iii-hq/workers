import path from 'node:path';

import {
  PARSE_EXTS,
  listWorkerRoots,
  walkFiles,
  parseFile,
  loadAcorn,
  readWorkerExempt,
  moduleStringConsts,
} from './scan-workers.mjs';
import { auditAsyncBoundaries } from './check-async-boundaries.mjs';
import { auditPayloadContractsAst } from './check-payload-contracts.mjs';
import { auditDurableState } from './check-durable-state.mjs';
import { auditWorkerIsolation } from './check-worker-isolation.mjs';

const AST_RULES = [
  auditAsyncBoundaries,
  auditPayloadContractsAst,
  auditDurableState,
  auditWorkerIsolation,
];

/** Single parse pass per source file; dispatches to AST rule auditors. */
export async function runSourceRules(scanRoot, options = {}) {
  const { acorn, walk } = await loadAcorn();
  const findings = [];

  const workerRoots = listWorkerRoots(scanRoot);
  const logs = [];

  for (const workerDir of workerRoots) {
    const exemptResult = readWorkerExempt(workerDir);
    if (!exemptResult.ok) {
      findings.push(...exemptResult.findings);
      continue;
    }
    logs.push(...exemptResult.logs);
    const workerOptions = {
      ...options,
      durableExempt: exemptResult.exempt,
      httpAllowlist: options.httpAllowlist,
    };

    for (const file of walkFiles(workerDir, { skipTests: true, skipSandbox: true })) {
      if (!PARSE_EXTS.has(path.extname(file))) continue;
      const rel = path.relative(scanRoot, file);
      let parsed;
      try {
        parsed = parseFile(acorn, file);
      } catch (e) {
        findings.push({
          rule_id: 'scan/parse',
          file: rel,
          line: 1,
          message: `parse error: ${e.message}`,
        });
        continue;
      }
      const ctx = {
        scanRoot,
        workerDir,
        workerRoots,
        file,
        rel,
        parsed,
        options: workerOptions,
        acorn,
        walk,
        findings,
        constMap: moduleStringConsts(parsed.ast),
      };
      for (const rule of AST_RULES) {
        rule(ctx);
      }
    }
  }
  return { findings, logs };
}
