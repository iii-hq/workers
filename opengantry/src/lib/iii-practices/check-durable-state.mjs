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

function isFsWriteCallee(node) {
  // fs.writeFile / fs.writeFileSync / fs.appendFile / fs.appendFileSync / fs.mkdirSync
  if (node.type === 'MemberExpression' && !node.computed) {
    if (node.object.type === 'Identifier' && node.object.name === 'fs') {
      return (
        node.property.type === 'Identifier' &&
        ['writeFile', 'writeFileSync', 'appendFile', 'appendFileSync', 'mkdirSync'].includes(
          node.property.name,
        )
      );
    }
  }
  if (node.type === 'Identifier') {
    return ['writeFile', 'writeFileSync', 'appendFile', 'appendFileSync', 'mkdirSync'].includes(
      node.name,
    );
  }
  return false;
}

function lhsIsGlobalOrProcess(node) {
  // global.x / globalThis.x / process.x / process.env.x
  let cur = node;
  if (cur.type === 'MemberExpression') {
    while (cur.type === 'MemberExpression') {
      if (cur.object.type === 'Identifier') {
        return ['global', 'globalThis', 'process'].includes(cur.object.name);
      }
      cur = cur.object;
    }
  }
  if (cur.type === 'Identifier') {
    return ['global', 'globalThis', 'process'].includes(cur.name);
  }
  return false;
}

function pathLooksAllowed(str, _workerDir) {
  if (typeof str !== 'string') return false;
  const norm = str.replace(/\\/g, '/');
  if (norm.split('/').includes('..')) return false;
  if (norm.includes('.gitagent/') || norm.endsWith('.gitagent') || norm.includes('/.gitagent/')) {
    return true;
  }
  if (norm.includes('.runtime/') || norm.includes('/.runtime')) return true;
  return false;
}

export async function checkDurableState(scanRoot) {
  const { acorn, walk } = await loadAcorn();
  const findings = [];
  const logs = [];

  for (const workerDir of listWorkerRoots(scanRoot)) {
    const exemptResult = readWorkerExempt(workerDir);
    if (!exemptResult.ok) {
      findings.push(...exemptResult.findings);
      continue;
    }
    logs.push(...exemptResult.logs);
    const exempt = exemptResult.exempt;
    const skipBags = exempt.has('durable-state/module-bags');

    for (const file of walkFiles(workerDir, { skipTests: true })) {
      if (!PARSE_EXTS.has(path.extname(file))) continue;
      let parsed;
      try {
        parsed = parseFile(acorn, file);
      } catch (e) {
        findings.push({
          rule_id: 'durable-state/parse',
          file: path.relative(scanRoot, file),
          line: 1,
          message: `parse error: ${e.message}`,
        });
        continue;
      }
      const rel = path.relative(scanRoot, file);
      const _constMap = moduleStringConsts(parsed.ast);

      // module-scope mutable bags
      if (!skipBags) {
        for (const stmt of parsed.ast.body) {
          if (stmt.type !== 'VariableDeclaration') continue;
          if (stmt.kind === 'const') continue;
          for (const d of stmt.declarations) {
            if (d.id?.type === 'Identifier') {
              findings.push({
                rule_id: 'durable-state/module-bags',
                file: rel,
                line: stmt.loc?.start?.line ?? 1,
                message: `module-scope mutable binding banned: ${stmt.kind} ${d.id.name}`,
              });
            }
          }
        }
      }

      walk.simple(parsed.ast, {
        AssignmentExpression(node) {
          if (lhsIsGlobalOrProcess(node.left)) {
            findings.push({
              rule_id: 'durable-state/global-process',
              file: rel,
              line: node.loc?.start?.line ?? 1,
              message: 'assignment to global/globalThis/process is banned (un-exemptible)',
            });
          }
        },
        UpdateExpression(node) {
          if (lhsIsGlobalOrProcess(node.argument)) {
            findings.push({
              rule_id: 'durable-state/global-process',
              file: rel,
              line: node.loc?.start?.line ?? 1,
              message: 'update of global/globalThis/process is banned (un-exemptible)',
            });
          }
        },
        CallExpression(node) {
          if (!isFsWriteCallee(node.callee)) return;
          const arg0 = node.arguments[0];
          const allowed =
            arg0?.type === 'Literal' &&
            typeof arg0.value === 'string' &&
            pathLooksAllowed(arg0.value, workerDir);
          if (!allowed) {
            findings.push({
              rule_id: 'durable-state/fs-writes',
              file: rel,
              line: node.loc?.start?.line ?? 1,
              message:
                'filesystem write outside allowlisted roots (.gitagent/, worker .runtime/) — un-exemptible',
            });
          }
        },
      });
    }
  }

  return { findings, logs };
}
