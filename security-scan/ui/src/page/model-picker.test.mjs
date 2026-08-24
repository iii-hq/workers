import assert from 'node:assert/strict'
import test from 'node:test'
import {
  FOLLOW_CHAT,
  modelPickerOptions,
  requestedModel,
  selectionIsStale,
} from './model-picker.js'

const catalog = [
  { key: 'anthropic::claude-fable-5', id: 'claude-fable-5', provider: 'anthropic', label: 'anthropic · Claude Fable 5' },
  { key: 'deepseek::deepseek-v4-pro', id: 'deepseek-v4-pro', provider: 'deepseek', label: 'deepseek · DeepSeek V4 Pro' },
]

test('lists the follow-chat entry first and then the catalog', () => {
  const options = modelPickerOptions(catalog, 'operator default: deepseek-v4-pro')
  assert.equal(options[0].value, FOLLOW_CHAT)
  assert.equal(options[0].label, 'follow chat · operator default: deepseek-v4-pro')
  assert.deepEqual(
    options.slice(1).map((option) => option.value),
    ['anthropic::claude-fable-5', 'deepseek::deepseek-v4-pro'],
  )
})

test('sends the pinned catalog key untouched so the worker splits it', () => {
  assert.equal(requestedModel('deepseek::deepseek-v4-pro', 'anthropic::claude-fable-5'), 'deepseek::deepseek-v4-pro')
})

test('follows the composer model when nothing is pinned', () => {
  assert.equal(requestedModel(FOLLOW_CHAT, 'anthropic::claude-fable-5'), 'anthropic::claude-fable-5')
})

test('sends no model at all when following a chat that has none', () => {
  assert.equal(requestedModel(FOLLOW_CHAT, null), null)
  assert.equal(requestedModel(FOLLOW_CHAT, '   '), null)
})

test('treats a pinned model missing from a loaded catalog as stale', () => {
  assert.equal(selectionIsStale(catalog, 'zai::glm-5.2'), true)
  assert.equal(selectionIsStale(catalog, 'deepseek::deepseek-v4-pro'), false)
})

test('never calls a selection stale while the catalog is still empty', () => {
  assert.equal(selectionIsStale([], 'deepseek::deepseek-v4-pro'), false)
  assert.equal(selectionIsStale([], FOLLOW_CHAT), false)
})
