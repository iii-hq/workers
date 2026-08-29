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
