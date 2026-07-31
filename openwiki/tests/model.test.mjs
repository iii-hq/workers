import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pickModel } from '../src/lib/model.mjs';

const MODELS = [
  { id: 'a', provider: 'p1', supports_tools: true },
  { id: 'b', provider: 'p2', supports_structured_output: true, supports_tools: true },
];

test('pickModel prefers the requested id when present', () => {
  const r = pickModel(MODELS, 'a');
  assert.equal(r.model, 'a');
  assert.equal(r.provider, 'p1');
  assert.equal(r.resolved, true);
});

test('pickModel falls back to a structured-output + tools model', () => {
  const r = pickModel(MODELS, 'missing');
  assert.equal(r.model, 'b');
  assert.equal(r.supports_structured_output, true);
});

test('pickModel returns unresolved when the catalog is empty', () => {
  const r = pickModel([], 'x');
  assert.equal(r.resolved, false);
  assert.equal(r.model, 'x');
});
