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

function isMutableContainerInit(init) {
  if (!init) return false;
  if (init.type === 'ObjectExpression' || init.type === 'ArrayExpression') return true;
  if (init.type === 'NewExpression' && init.callee?.type === 'Identifier') {
    return ['Map', 'Set', 'WeakMap', 'WeakSet'].includes(init.callee.name);
  }
  return false;
}

/** @param {import('./run-source-rules.mjs').AuditCtx} ctx */
export function auditDurableState(ctx) {
  const { parsed, rel, walk, findings, workerDir, options } = ctx;
  const skipBags = options.durableExempt?.has('durable-state/module-bags') ?? false;

  if (!skipBags) {
    for (const top of parsed.ast.body) {
      const stmt = top.type === 'ExportNamedDeclaration' ? top.declaration : top;
      if (!stmt || stmt.type !== 'VariableDeclaration') continue;
      for (const d of stmt.declarations) {
        if (d.id?.type !== 'Identifier') continue;
        const isLetVar = stmt.kind === 'let' || stmt.kind === 'var';
        const isConstBag = stmt.kind === 'const' && isMutableContainerInit(d.init);
        if (isLetVar || isConstBag) {
          findings.push({
            rule_id: 'durable-state/module-bags',
            file: rel,
            line: stmt.loc?.start?.line ?? 1,
            message: `module-scope mutable container banned: ${stmt.kind} ${d.id.name}`,
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

/** @deprecated use runPracticesScan */
export async function checkDurableState(scanRoot) {
  const { runPracticesScan } = await import('./scan.mjs');
  const { findings, logs } = await runPracticesScan(scanRoot);
  return {
    findings: findings.filter((f) => f.rule_id.startsWith('durable-state/')),
    logs,
  };
}
