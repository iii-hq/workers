import path from 'node:path';

function resolveRelative(fromFile, spec) {
  if (!spec.startsWith('.')) return null;
  return path.normalize(path.join(path.dirname(fromFile), spec));
}

function workerOfPath(absFile, workerRoots) {
  for (const root of workerRoots) {
    const rel = path.relative(root, absFile);
    if (!rel.startsWith('..') && !path.isAbsolute(rel)) {
      return path.basename(root);
    }
  }
  return null;
}

/** @param {import('./run-source-rules.mjs').AuditCtx} ctx */
export function auditWorkerIsolation(ctx) {
  const { parsed, rel, file, walk, findings, workerDir, workerRoots } = ctx;
  const selfName = path.basename(workerDir);

  function checkSpec(spec, line) {
    if (typeof spec !== 'string') return;
    const target = resolveRelative(file, spec);
    if (!target) return;
    const candidates = [
      target,
      `${target}.js`,
      `${target}.mjs`,
      `${target}.ts`,
      path.join(target, 'index.js'),
      path.join(target, 'index.ts'),
    ];
    for (const cand of candidates) {
      const other = workerOfPath(cand, workerRoots);
      if (other && other !== selfName) {
        findings.push({
          rule_id: 'isolation/cross-worker',
          file: rel,
          line,
          message: `cross-worker import from ${selfName} into ${other}: ${spec}`,
        });
        return;
      }
    }
  }

  walk.simple(parsed.ast, {
    ImportDeclaration(node) {
      checkSpec(node.source?.value, node.loc?.start?.line ?? 1);
    },
    CallExpression(node) {
      if (node.callee.type === 'Import') {
        const arg = node.arguments[0];
        if (!arg || arg.type !== 'Literal' || typeof arg.value !== 'string') {
          findings.push({
            rule_id: 'isolation/dynamic-import',
            file: rel,
            line: node.loc?.start?.line ?? 1,
            message: 'computed dynamic import() banned (string literal required)',
          });
          return;
        }
        checkSpec(arg.value, node.loc?.start?.line ?? 1);
      }
    },
  });
}

/** @deprecated use runSourceRules */
export async function checkWorkerIsolation(scanRoot) {
  const { runSourceRules } = await import('./run-source-rules.mjs');
  const all = await runSourceRules(scanRoot, {});
  return all.filter((f) => f.rule_id.startsWith('isolation/'));
}
