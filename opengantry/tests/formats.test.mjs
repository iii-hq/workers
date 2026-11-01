/**
 * Publish gate, enforced locally. Candidate smoke boots the released
 * bundle and runs `collect_worker_interface.py --assert-typed-schemas`, which
 * fails the release if any registered function's request or response schema
 * lacks a schema-defining keyword — `z.unknown()` compiles to `{}` and trips it.
 * That check only runs *after* the tag, the GitHub Release, and the tarball
 * upload, so catch it here instead.
 */
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { formatFor, FUNCTION_FORMATS } from '../src/formats.js';

// Keep in sync with SCHEMA_DEFINING_KEYS in
// .github/scripts/collect_worker_interface.py.
const SCHEMA_DEFINING_KEYS = [
  'type',
  'properties',
  '$ref',
  'allOf',
  'anyOf',
  'oneOf',
  'enum',
  'items',
  'const',
];

test('every registered function publishes typed request and response schemas', () => {
  const violations = [];
  for (const [functionId, formats] of Object.entries(FUNCTION_FORMATS)) {
    for (const field of ['request', 'response']) {
      const schema = formatFor(formats[field]);
      const typed =
        schema !== null &&
        typeof schema === 'object' &&
        SCHEMA_DEFINING_KEYS.some((key) => key in schema);
      if (!typed) {
        violations.push(`${functionId}.${field}_schema`);
      }
    }
  }
  assert.deepEqual(
    violations,
    [],
    `untyped schemas would fail --assert-typed-schemas at publish: ${violations.join(', ')}`,
  );
});

test('gantry::on-function-registration publishes the engine input, observe-v2 fields included', () => {
  const request = formatFor(FUNCTION_FORMATS['gantry::on-function-registration'].request);
  assert.equal(request.type, 'object');
  assert.deepEqual(request.required, ['function_id']);
  assert.deepEqual(request.properties.private, { type: 'boolean' });
  assert.deepEqual(request.properties.observable, { type: 'boolean' });
  assert.equal(request.properties.args_format.type, 'object');
  for (const key of ['description', 'metadata', 'namespace', 'context']) {
    assert.ok(key in request.properties, `request_format names ${key}`);
  }
  // The engine owns the envelope: fields it adds later must not fail the schema.
  assert.notEqual(request.additionalProperties, false);

  const response = formatFor(FUNCTION_FORMATS['gantry::on-function-registration'].response);
  assert.deepEqual(Object.keys(response.properties), ['function_id']);
});
