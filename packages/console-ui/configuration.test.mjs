import assert from 'node:assert/strict'
import { test } from 'node:test'
import { resolveConfigurationId } from './configuration.mjs'

test('uses the addressed worker and preserves explicit or generated IDs', async () => {
  for (const id of ['orders-voice-0123456789abcdef', 'custom-entry']) {
    const calls = []
    assert.equal(await resolveConfigurationId({ trigger: async (...args) => {
      calls.push(args)
      return { id }
    } }, 'voice'), id)
    assert.deepEqual(calls, [['voice::configuration-id', {}, { timeoutMs: 5000 }]])
  }
})

test('never falls back to a global entry on errors or malformed identities', async () => {
  for (const response of [null, {}, { id: '' }, { id: '   ' }, { id: 123 }]) {
    await assert.rejects(resolveConfigurationId({ trigger: async () => response }, 'ide'), /invalid configuration identity/)
  }
  const error = new Error('function_not_found')
  await assert.rejects(resolveConfigurationId({ trigger: async () => { throw error } }, 'ide'), (caught) => caught === error)
})

test('does not cache an identity across worker replacement', async () => {
  let id = 'first'
  const iii = { trigger: async () => ({ id }) }
  assert.equal(await resolveConfigurationId(iii, 'voice'), 'first')
  id = 'second'
  assert.equal(await resolveConfigurationId(iii, 'voice'), 'second')
})
