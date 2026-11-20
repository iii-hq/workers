import assert from 'node:assert/strict'
import test from 'node:test'
import esbuild from 'esbuild'

// The Run panel checks a body against the function's request schema before
// the call. A worker handed the wrong type may never answer, so the obvious
// mistakes must be caught here, and only the obvious ones: a loose schema
// must never block a call the worker would accept.
async function loadSchema() {
  const result = await esbuild.build({
    entryPoints: ['src/catalog/schema.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

const chartGet = {
  type: 'object',
  properties: {
    id: { type: 'integer' },
    weeks: { type: 'integer' },
    kind: { type: 'string', enum: ['line', 'bar'] },
    tags: { type: 'array', items: { type: 'string' } },
    range: { type: 'object', properties: { from: { type: 'string' } }, required: ['from'] },
  },
  required: ['id'],
}

test('a string where the schema wants an integer is named before the call', async () => {
  const { schemaProblems } = await loadSchema()
  assert.deepEqual(schemaProblems(chartGet, { id: 0, weeks: 'test' }), ['weeks must be an integer, not "test"'])
})

test('a body the schema accepts has no problems', async () => {
  const { schemaProblems } = await loadSchema()
  assert.deepEqual(schemaProblems(chartGet, { id: 3, weeks: 4, kind: 'bar', tags: ['a'] }), [])
})

test('a missing required field is named, an optional one is not', async () => {
  const { schemaProblems } = await loadSchema()
  assert.deepEqual(schemaProblems(chartGet, {}), ['id is required'])
})

test('enum members, nested objects and array items are checked', async () => {
  const { schemaProblems } = await loadSchema()
  assert.deepEqual(schemaProblems(chartGet, { id: 1, kind: 'pie' }), ['kind must be one of "line", "bar"'])
  assert.deepEqual(schemaProblems(chartGet, { id: 1, range: {} }), ['range.from is required'])
  assert.deepEqual(schemaProblems(chartGet, { id: 1, tags: ['a', 2] }), ['tags[1] must be a string, not 2'])
})

test('a fractional number is not an integer, but is a number', async () => {
  const { schemaProblems } = await loadSchema()
  assert.deepEqual(schemaProblems({ type: 'object', properties: { n: { type: 'integer' } } }, { n: 1.5 }), [
    'n must be an integer, not 1.5',
  ])
  assert.deepEqual(schemaProblems({ type: 'object', properties: { n: { type: 'number' } } }, { n: 1.5 }), [])
})

test('a union type and a null member read as the schema says', async () => {
  const { schemaProblems } = await loadSchema()
  const schema = { type: 'object', properties: { w: { type: ['integer', 'null'] } } }
  assert.deepEqual(schemaProblems(schema, { w: null }), [])
  assert.deepEqual(schemaProblems(schema, { w: 'x' }), ['w must be an integer or null, not "x"'])
})

test('an anyOf of plain types reads as one union, and passes when a branch takes the value', async () => {
  const { schemaProblems } = await loadSchema()
  const schema = { type: 'object', properties: { limit: { anyOf: [{ type: 'integer' }, { type: 'null' }] } } }
  assert.deepEqual(schemaProblems(schema, { limit: null }), [])
  assert.deepEqual(schemaProblems(schema, { limit: 3 }), [])
  assert.deepEqual(schemaProblems(schema, { limit: 'test' }), ['limit must be an integer or null, not "test"'])
  const objects = { anyOf: [{ type: 'object', required: ['a'] }, { type: 'object', properties: { b: { type: 'string' } } }] }
  assert.deepEqual(schemaProblems(objects, { b: 'x' }), [])
})

test('what the schema leaves open passes', async () => {
  const { schemaProblems } = await loadSchema()
  assert.deepEqual(schemaProblems(undefined, { anything: 1 }), [])
  assert.deepEqual(schemaProblems({ type: 'object' }, { anything: 'goes' }), [])
  assert.deepEqual(schemaProblems({ anyOf: [{ $ref: '#/a' }, { $ref: '#/b' }] }, { x: 1 }), [])
  assert.deepEqual(schemaProblems({ type: 'object', properties: { r: { $ref: '#/defs/x' } } }, { r: 'y' }), [])
})
