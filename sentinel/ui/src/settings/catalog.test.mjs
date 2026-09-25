import assert from 'node:assert/strict'
import { test } from 'node:test'
import { catalogKey, modelGroups, readCatalog, splitKey } from './catalog.js'

const CATALOG = readCatalog({
  models: [
    { id: 'claude-sonnet-5', provider: 'anthropic', display_name: 'Claude Sonnet 5' },
    { id: 'deepseek-v4-pro', provider: 'deepseek' },
    { id: 'gpt-5.2', provider: 'openai', display_name: 'GPT-5.2' },
  ],
})

test('a row without an id or a provider cannot be addressed and is dropped', () => {
  const catalog = readCatalog({
    models: [{ id: 'a', provider: 'p' }, { id: '', provider: 'p' }, { id: 'b' }, null, 'nope'],
  })
  assert.deepEqual(
    catalog.map((model) => model.key),
    ['p::a'],
  )
})

test('an unreachable router is an empty catalog, not a crash', () => {
  assert.deepEqual(readCatalog(null), [])
  assert.deepEqual(readCatalog({}), [])
  assert.deepEqual(readCatalog({ models: 'soon' }), [])
})

test('a row without a display name falls back to its id', () => {
  const deepseek = CATALOG.find((model) => model.provider === 'deepseek')
  assert.equal(deepseek?.label, 'deepseek-v4-pro')
})

test('the key joins and splits back into the two fields the worker stores', () => {
  assert.equal(catalogKey('claude-sonnet-5', 'anthropic'), 'anthropic::claude-sonnet-5')
  assert.deepEqual(splitKey('anthropic::claude-sonnet-5'), {
    model: 'claude-sonnet-5',
    provider: 'anthropic',
  })
})

test('a raw id with no provider survives the round trip', () => {
  assert.equal(catalogKey('some-model', ''), 'some-model')
  assert.deepEqual(splitKey('some-model'), { model: 'some-model', provider: '' })
})

test('nothing configured is an empty key, not a half one', () => {
  assert.equal(catalogKey('', 'anthropic'), '')
  assert.equal(catalogKey(undefined, undefined), '')
})

test('the picker groups by provider', () => {
  const groups = modelGroups(CATALOG, 'anthropic::claude-sonnet-5')
  assert.deepEqual(
    groups.map((group) => group.label),
    ['anthropic', 'deepseek', 'openai'],
  )
  assert.deepEqual(groups[0].options[0], {
    value: 'anthropic::claude-sonnet-5',
    label: 'Claude Sonnet 5',
  })
})

test('a configured model the router no longer offers is kept, and says why', () => {
  const groups = modelGroups(CATALOG, 'zai::glm-9')
  assert.equal(groups[0].label, 'configured')
  assert.equal(groups[0].options[0].value, 'zai::glm-9')
  assert.equal(groups[0].options[0].description, 'not offered by the router right now')
  assert.equal(groups.length, 4, 'and the catalog is still offered beside it')
})

test('before the catalog loads, the configured model is not accused of being gone', () => {
  const groups = modelGroups([], 'zai::glm-9')
  assert.equal(groups[0].options[0].description, 'the router catalog is not loaded')
})

test('nothing configured adds no group of its own', () => {
  assert.deepEqual(
    modelGroups(CATALOG, '').map((group) => group.label),
    ['anthropic', 'deepseek', 'openai'],
  )
})
