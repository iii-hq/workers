import path from 'node:path';
import { workerNameOf } from './scan-workers.mjs';
import { HTTP_PRAGMA } from './allowlist.mjs';

const HTTP_MODULES = new Set([
  'axios',
  'node-fetch',
  'undici',
  'got',
  'superagent',
  'http',
  'https',
  'node:http',
  'node:https',
]);

const HTTP_CALLEES = new Set(['fetch', 'axios']);

const FUNCTION_TYPES = new Set([
  'FunctionDeclaration',
  'FunctionExpression',
  'ArrowFunctionExpression',
]);

function isHttpMember(node) {
  if (node.type !== 'MemberExpression' || node.computed) return false;
  if (node.object.type !== 'Identifier') return false;
  if (node.object.name !== 'http' && node.object.name !== 'https') return false;
  return node.property.type === 'Identifier' && ['request', 'get'].includes(node.property.name);
}

function innermostEnclosingFunction(ancestors) {
  for (let i = ancestors.length - 1; i >= 0; i--) {
    const node = ancestors[i];
    if (FUNCTION_TYPES.has(node.type)) return node;
  }
  return null;
}

/** Leading block comment on the enclosing function must contain HTTP_PRAGMA. */
function functionHasHttpPragma(fnNode, comments) {
  if (!fnNode) return false;
  const start = fnNode.start ?? fnNode.body?.start ?? 0;
  const leading = comments.filter((c) => c.block && c.end <= start).sort((a, b) => b.end - a.end);
  for (const c of leading) {
    if (start - c.end > 80) break;
    if (c.text.includes(HTTP_PRAGMA)) return true;
  }
  return false;
}

/**
 * HTTP client ban with planner allowlist + function pragma ratchet (MSN-0161).
 * @param {import('./run-source-rules.mjs').AuditCtx} ctx
 */
export function auditAsyncBoundaries(ctx) {
  const { parsed, rel, scanRoot, file, walk, options, findings } = ctx;
  const httpAllowlist = options.httpAllowlist ?? new Set();
  const { ast, comments } = parsed;
  const workerName = workerNameOf(file, scanRoot);

  walk.ancestor(ast, {
    ImportDeclaration(node) {
      const src = node.source?.value;
      if (typeof src === 'string' && HTTP_MODULES.has(src)) {
        findings.push({
          rule_id: 'async/http-import',
          file: rel,
          line: node.loc?.start?.line ?? 1,
          message: `HTTP client import banned: ${src}`,
        });
      }
    },
    CallExpression(node, _state, ancestors) {
      const callee = node.callee;
      const line = node.loc?.start?.line ?? 1;

      const reportCall = (message) => {
        const fn = innermostEnclosingFunction(ancestors);
        const hasPragma = functionHasHttpPragma(fn, comments);
        if (!hasPragma) {
          findings.push({
            rule_id: 'async/http-call',
            file: rel,
            line,
            message,
          });
          return;
        }
        if (!httpAllowlist.has(workerName)) {
          findings.push({
            rule_id: 'async/http-pragma-denied',
            file: rel,
            line,
            message: `/* ${HTTP_PRAGMA} */ present but worker "${workerName}" is not on planner http_connector_workers allowlist`,
          });
        }
      };

      if (callee.type === 'Identifier' && HTTP_CALLEES.has(callee.name)) {
        reportCall(`HTTP client call banned: ${callee.name}(...)`);
      }
      if (isHttpMember(callee)) {
        reportCall('HTTP client call banned: http(s).request/get');
      }
      if (callee.type === 'Import') {
        const arg = node.arguments[0];
        if (!arg || arg.type !== 'Literal' || typeof arg.value !== 'string') {
          findings.push({
            rule_id: 'async/dynamic-import',
            file: rel,
            line,
            message: 'computed dynamic import() banned (string literal path required)',
          });
        } else if (HTTP_MODULES.has(arg.value)) {
          reportCall(`HTTP client dynamic import banned: ${arg.value}`);
        }
      }
    },
  });
}

/** @deprecated use runSourceRules */
export async function checkAsyncBoundaries(scanRoot, options = {}) {
  const { runSourceRules } = await import('./run-source-rules.mjs');
  const { findings } = await runSourceRules(scanRoot, options);
  return findings.filter((f) => f.rule_id.startsWith('async/'));
}
