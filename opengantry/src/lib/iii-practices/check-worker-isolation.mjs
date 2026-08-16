import path from 'node:path';
import { PARSE_EXTS, listWorkerRoots, walkFiles, parseFile, loadAcorn } from './scan-workers.mjs';

function resolveRelative(fromFile, spec) {
  if (!spec.startsWith('.')) return null;
  return path.normalize(path.join(path.dirname(fromFile), spec));
}

function workerOfPath(absFile, _scanRoot, workerRoots) {
  for (const root of workerRoots) {
    const rel = path.relative(root, absFile);
    if (!rel.startsWith('..') && !path.isAbsolute(rel)) {
      return path.basename(root);
    }
  }
  return null;
}

export async function checkWorkerIsolation(scanRoot) {
  const { acorn, walk } = await loadAcorn();
  const findings = [];
  const workerRoots = listWorkerRoots(scanRoot);

  for (const workerDir of workerRoots) {
    const selfName = path.basename(workerDir);
    for (const file of walkFiles(workerDir, { skipTests: true })) {
      if (!PARSE_EXTS.has(path.extname(file))) continue;
      let parsed;
      try {
        parsed = parseFile(acorn, file);
      } catch (e) {
        findings.push({
          rule_id: 'isolation/parse',
          file: path.relative(scanRoot, file),
          line: 1,
          message: `parse error: ${e.message}`,
        });
        continue;
      }
      const rel = path.relative(scanRoot, file);

      function checkSpec(spec, line) {
        if (typeof spec !== 'string') return;
        const target = resolveRelative(file, spec);
        if (!target) return;
        // try with extensions
        const candidates = [
          target,
          `${target}.js`,
          `${target}.mjs`,
          `${target}.ts`,
          path.join(target, 'index.js'),
          path.join(target, 'index.ts'),
        ];
        for (const cand of candidates) {
          const other = workerOfPath(cand, scanRoot, workerRoots);
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
  }
  return findings;
}
