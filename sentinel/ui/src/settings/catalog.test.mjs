import assert from 'node:assert/strict'
import { test } from 'node:test'
import { catalogKey, readCatalog, splitKey, withStoredModel } from './catalog.js'

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
    catalog.map((model) => model.id),
    ['p::a'],
  )
})

test('an unreachable router is an empty catalog, not a crash', () => {
  assert.deepEqual(readCatalog(null), [])
  assert.deepEqual(readCatalog({}), [])
  assert.deepEqual(readCatalog({ models: 'soon' }), [])
})

test('a row without a display name falls back to its id', () => {
  const deepseek = CATALOG.find((model) => model.id === 'deepseek::deepseek-v4-pro')
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

test('an option is keyed like the stored pair and named for people', () => {
  assert.deepEqual(
    CATALOG.map((model) => [model.id, model.label]),
    [
      ['anthropic::claude-sonnet-5', 'Claude Sonnet 5'],
      ['deepseek::deepseek-v4-pro', 'deepseek-v4-pro'],
      ['openai::gpt-5.2', 'GPT-5.2'],
    ],
  )
})

test('a configured model the router no longer offers is kept beside the catalog', () => {
  const options = withStoredModel(CATALOG, 'zai::glm-9')
  assert.equal(options.length, 4)
  assert.deepEqual(options.at(-1), { id: 'zai::glm-9', label: 'glm-9' })
})

test('an offered or empty selection adds nothing', () => {
  assert.equal(withStoredModel(CATALOG, 'openai::gpt-5.2'), CATALOG)
  assert.equal(withStoredModel(CATALOG, ''), CATALOG)
})
