/**
 * Shared walker for iii-architecture lint profile.
 * Worker = immediate child of scanRoot with package.json.
 */
import fs from 'node:fs';
import path from 'node:path';
import { createRequire, stripTypeScriptTypes } from 'node:module';

const require = createRequire(import.meta.url);

export const PARSE_EXTS = new Set(['.js', '.mjs', '.ts', '.tsx', '.mts', '.cts']);
export const TS_EXTS = new Set(['.ts', '.tsx', '.mts', '.cts']);

export const EXEMPTIBLE_RULES = new Set(['durable-state/module-bags']);
export const FORBIDDEN_EXEMPT_RULES = new Set([
  'durable-state/global-process',
  'durable-state/fs-writes',
]);

export function preflightDeps() {
  if (process.env.GANTRY_III_ARCH_FORCE_FATAL === '1') {
    const err = new Error(
      'FATAL: EXIT 2 — scanner could not run; this is NOT an architecture violation. Check examples/iii-integration deps. (GANTRY_III_ARCH_FORCE_FATAL=1)',
    );
    err.code = 'FORCE_FATAL';
    throw err;
  }
  for (const name of ['acorn', 'acorn-walk', 'ajv']) {
    try {
      require.resolve(name);
    } catch {
      const err = new Error(
        `FATAL: EXIT 2 — scanner could not run; this is NOT an architecture violation. Check examples/iii-integration deps. (missing ${name})`,
      );
      err.code = 'MISSING_DEP';
      throw err;
    }
  }
}

export async function loadAcorn() {
  const acorn = await import('acorn');
  const walk = await import('acorn-walk');
  return { acorn, walk };
}

export function isSingleWorkerScanRoot(scanRoot) {
  return fs.existsSync(path.join(scanRoot, 'package.json'));
}

export function listWorkerRoots(scanRoot) {
  if (!fs.existsSync(scanRoot)) return [];
  if (isSingleWorkerScanRoot(scanRoot)) return [scanRoot];
  return fs
    .readdirSync(scanRoot, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => path.join(scanRoot, d.name))
    .filter((dir) => fs.existsSync(path.join(dir, 'package.json')));
}

export function findOrphanSourceDirs(scanRoot) {
  const findings = [];
  if (!fs.existsSync(scanRoot)) return findings;
  if (isSingleWorkerScanRoot(scanRoot)) return findings;
  for (const ent of fs.readdirSync(scanRoot, { withFileTypes: true })) {
    if (!ent.isDirectory()) continue;
    const dir = path.join(scanRoot, ent.name);
    if (fs.existsSync(path.join(dir, 'package.json'))) continue;
    const hasSource = walkFiles(dir).some((f) => PARSE_EXTS.has(path.extname(f)));
    if (hasSource) {
      findings.push({
        rule_id: 'worker/package-json',
        file: path.relative(scanRoot, dir) || ent.name,
        line: 1,
        message: `directory under workers/ has source but no package.json (not a worker): ${ent.name}`,
      });
    }
  }
  return findings;
}

export function walkFiles(root, options = {}) {
  const out = [];
  function rec(dir) {
    if (!fs.existsSync(dir)) return;
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.name === 'node_modules' || ent.name === '.git' || ent.name === 'dist') continue;
      if (options.skipTests && ent.isDirectory() && ent.name === 'tests') continue;
      if (ent.name === 'sandbox.mjs') continue;
      const p = path.join(dir, ent.name);
      if (ent.isDirectory()) rec(p);
      else out.push(p);
    }
  }
  rec(root);
  return out;
}

export function transpileForParse(filePath, source) {
  const ext = path.extname(filePath);
  if (!TS_EXTS.has(ext)) return source;
  try {
    const ts = require('typescript');
    return ts.transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.ESNext,
        target: ts.ScriptTarget.ES2022,
        jsx: ext === '.tsx' ? ts.JsxEmit.Preserve : ts.JsxEmit.None,
      },
      fileName: filePath,
      reportDiagnostics: false,
    }).outputText;
  } catch {
    return stripTypeScriptTypes(source);
  }
}

export function parseFile(acorn, filePath) {
  const raw = fs.readFileSync(filePath, 'utf8');
  const code = transpileForParse(filePath, raw);
  const comments = [];
  const ast = acorn.parse(code, {
    ecmaVersion: 'latest',
    sourceType: 'module',
    locations: true,
    allowHashBang: true,
    onComment: (block, text, start, end, loc) => {
      comments.push({ block, text, start, end, loc });
    },
  });
  return { code, ast, comments };
}

/** Module-scope const Name = 'literal' map. */
export function moduleStringConsts(ast) {
  const map = new Map();
  for (const stmt of ast.body) {
    if (stmt.type !== 'VariableDeclaration' || stmt.kind !== 'const') continue;
    for (const d of stmt.declarations) {
      if (d.id?.type !== 'Identifier') continue;
      if (d.init?.type === 'Literal' && typeof d.init.value === 'string') {
        map.set(d.id.name, d.init.value);
      }
    }
  }
  return map;
}

export function resolveStringExpr(node, constMap) {
  if (!node) return null;
  if (node.type === 'Literal' && typeof node.value === 'string') return node.value;
  if (node.type === 'Identifier' && constMap.has(node.name)) return constMap.get(node.name);
  return null;
}

export function readWorkerExempt(workerDir) {
  const pkgPath = path.join(workerDir, 'package.json');
  let pkg;
  try {
    pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
  } catch (e) {
    return {
      ok: false,
      findings: [
        {
          rule_id: 'exempt/config',
          file: `${path.basename(workerDir)}/package.json`,
          line: 1,
          message: `invalid package.json: ${e.message}`,
        },
      ],
      exempt: new Set(),
      logs: [],
    };
  }
  if (!pkg || typeof pkg !== 'object' || Array.isArray(pkg)) {
    return {
      ok: false,
      findings: [
        {
          rule_id: 'exempt/config',
          file: `${path.basename(workerDir)}/package.json`,
          line: 1,
          message: 'package.json root must be a JSON object',
        },
      ],
      exempt: new Set(),
      logs: [],
    };
  }
  const raw = pkg.iii_architecture?.exempt;
  if (raw == null) return { ok: true, exempt: new Set(), logs: [] };
  if (!Array.isArray(raw)) {
    return {
      ok: false,
      findings: [
        {
          rule_id: 'exempt/config',
          file: `${path.basename(workerDir)}/package.json`,
          line: 1,
          message: 'iii_architecture.exempt must be an array',
        },
      ],
      exempt: new Set(),
      logs: [],
    };
  }
  const findings = [];
  const exempt = new Set();
  const logs = [];
  for (const id of raw) {
    if (typeof id !== 'string') {
      findings.push({
        rule_id: 'exempt/config',
        file: `${path.basename(workerDir)}/package.json`,
        line: 1,
        message: `invalid exempt entry (not a string): ${JSON.stringify(id)}`,
      });
      continue;
    }
    if (FORBIDDEN_EXEMPT_RULES.has(id) || !EXEMPTIBLE_RULES.has(id)) {
      findings.push({
        rule_id: 'exempt/config',
        file: `${path.basename(workerDir)}/package.json`,
        line: 1,
        message: `forbidden or unknown exempt id rejected at config load: ${id}`,
      });
      continue;
    }
    exempt.add(id);
    logs.push(`iii-architecture: exempt worker=${path.basename(workerDir)} rule=${id}`);
  }
  if (findings.length) return { ok: false, findings, exempt: new Set(), logs: [] };
  return { ok: true, exempt, logs };
}

export function workerNameOf(file, scanRoot) {
  if (isSingleWorkerScanRoot(scanRoot)) return path.basename(scanRoot);
  const rel = path.relative(scanRoot, file);
  const top = rel.split(path.sep)[0];
  return top;
}

export function schemaFileName(functionId) {
  return `${functionId.replaceAll('::', '__')}.json`;
}
