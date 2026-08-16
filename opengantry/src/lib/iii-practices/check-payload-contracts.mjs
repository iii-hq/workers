import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import {
  PARSE_EXTS,
  listWorkerRoots,
  walkFiles,
  parseFile,
  loadAcorn,
  moduleStringConsts,
  resolveStringExpr,
} from './scan-workers.mjs';

const require = createRequire(import.meta.url);
const COMPOSITION_KEYS = new Set(['allOf', 'oneOf', 'anyOf', '$ref', 'unevaluatedProperties']);

function walkSchemaObject(node, pathLabel, findings, file) {
  if (!node || typeof node !== 'object' || Array.isArray(node)) return;
  for (const key of Object.keys(node)) {
    if (COMPOSITION_KEYS.has(key)) {
      findings.push({
        rule_id: 'payload/schema-composition',
        file,
        line: 1,
        message: `schema composition keyword banned at ${pathLabel}: ${key}`,
      });
    }
  }
  if (node.type === 'object' || (!node.type && node.properties)) {
    if (node.additionalProperties !== false) {
      findings.push({
        rule_id: 'payload/additionalProperties',
        file,
        line: 1,
        message: `object schema at ${pathLabel} requires additionalProperties: false`,
      });
    }
    const props = node.properties;
    if (!props || typeof props !== 'object' || Object.keys(props).length === 0) {
      if (pathLabel === '$' || node.type === 'object') {
        findings.push({
          rule_id: 'payload/properties',
          file,
          line: 1,
          message: `object schema at ${pathLabel} must declare at least one named property`,
        });
      }
    } else {
      for (const [name, prop] of Object.entries(props)) {
        if (!prop || typeof prop !== 'object' || !prop.type) {
          findings.push({
            rule_id: 'payload/property-type',
            file,
            line: 1,
            message: `property ${pathLabel}.${name} must have type`,
          });
        }
        walkSchemaObject(prop, `${pathLabel}.${name}`, findings, file);
      }
    }
  }
  if (Array.isArray(node.items)) {
    for (let i = 0; i < node.items.length; i++) {
      walkSchemaObject(node.items[i], `${pathLabel}.items[${i}]`, findings, file);
    }
  } else if (node.items) {
    walkSchemaObject(node.items, `${pathLabel}.items`, findings, file);
  }
}

function validateSchemaFile(Ajv, schemaPath, relSchema, findings) {
  let raw;
  try {
    raw = JSON.parse(fs.readFileSync(schemaPath, 'utf8'));
  } catch (e) {
    findings.push({
      rule_id: 'payload/schema-parse',
      file: relSchema,
      line: 1,
      message: `invalid JSON schema: ${e.message}`,
    });
    return;
  }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    findings.push({
      rule_id: 'payload/schema-parse',
      file: relSchema,
      line: 1,
      message: 'schema root must be a JSON object',
    });
    return;
  }
  if (raw.$schema !== 'http://json-schema.org/draft-07/schema#') {
    findings.push({
      rule_id: 'payload/schema-draft',
      file: relSchema,
      line: 1,
      message: 'schema must set "$schema": "http://json-schema.org/draft-07/schema#"',
    });
  }
  const ajv = new Ajv({ strict: false, allErrors: true });
  try {
    ajv.compile(raw);
  } catch (e) {
    findings.push({
      rule_id: 'payload/schema-meta',
      file: relSchema,
      line: 1,
      message: `ajv rejected schema: ${e.message}`,
    });
  }
  walkSchemaObject(raw, '$', findings, relSchema);
}

function isRegisterTriggerTypeCall(node) {
  const c = node.callee;
  if (c.type === 'MemberExpression' && !c.computed && c.property.type === 'Identifier') {
    return c.property.name === 'registerTriggerType';
  }
  if (c.type === 'Identifier') {
    return c.name === 'registerTriggerType';
  }
  return false;
}

function isRegisterFunctionCall(node) {
  const c = node.callee;
  if (c.type === 'MemberExpression' && !c.computed && c.property.type === 'Identifier') {
    return c.property.name === 'registerFunction' || c.property.name === 'register_function';
  }
  if (c.type === 'Identifier') {
    return c.name === 'registerFunction' || c.name === 'register_function';
  }
  return false;
}

function objectHasIdentProp(node, name) {
  if (!node || node.type !== 'ObjectExpression') return false;
  return node.properties.some(
    (p) =>
      p.type === 'Property' &&
      !p.computed &&
      ((p.key.type === 'Identifier' && p.key.name === name) ||
        (p.key.type === 'Literal' && p.key.value === name)),
  );
}

export async function checkPayloadContracts(scanRoot) {
  const { acorn, walk } = await loadAcorn();
  const Ajv = require('ajv');
  const findings = [];

  for (const workerDir of listWorkerRoots(scanRoot)) {
    const schemasDir = path.join(workerDir, 'schemas');
    if (fs.existsSync(schemasDir)) {
      for (const file of walkFiles(schemasDir)) {
        if (path.extname(file) !== '.json') continue;
        validateSchemaFile(Ajv, file, path.relative(scanRoot, file), findings);
      }
    }
    for (const file of walkFiles(workerDir, { skipTests: true })) {
      if (!PARSE_EXTS.has(path.extname(file))) continue;
      let parsed;
      try {
        parsed = parseFile(acorn, file);
      } catch (e) {
        findings.push({
          rule_id: 'payload/parse',
          file: path.relative(scanRoot, file),
          line: 1,
          message: `parse error: ${e.message}`,
        });
        continue;
      }
      const constMap = moduleStringConsts(parsed.ast);
      const rel = path.relative(scanRoot, file);

      walk.simple(parsed.ast, {
        CallExpression(node) {
          if (isRegisterTriggerTypeCall(node) && node.arguments.length < 2) {
            findings.push({
              rule_id: 'runtime/register-trigger-type',
              file: rel,
              line: node.loc?.start?.line ?? 1,
              message:
                'registerTriggerType requires a trigger-type handler with registerTrigger and unregisterTrigger',
            });
          }
          if (!isRegisterFunctionCall(node)) return;
          const idArg = node.arguments[0];
          const resolved = resolveStringExpr(idArg, constMap);
          if (resolved == null) {
            findings.push({
              rule_id: 'payload/register-id',
              file: rel,
              line: node.loc?.start?.line ?? 1,
              message:
                'registerFunction id must be a string literal or module-scope const string (imported bindings fail)',
            });
            return;
          }
          const opts = node.arguments[2];
          if (
            !objectHasIdentProp(opts, 'request_format') ||
            !objectHasIdentProp(opts, 'response_format')
          ) {
            findings.push({
              rule_id: 'payload/request-response-format',
              file: rel,
              line: node.loc?.start?.line ?? 1,
              message: `registerFunction ${resolved} must pass request_format and response_format`,
            });
          }
        },
      });
    }
  }
  return findings;
}
