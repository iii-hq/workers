import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { persistenceModeFor, withPersistenceMode } from '../src/configuration/model.ts'

describe('state persistence configuration', () => {
  it('treats an omitted store method as file based', () => {
    assert.equal(persistenceModeFor(undefined), 'file_based')
    assert.equal(persistenceModeFor('file_based'), 'file_based')
    assert.equal(persistenceModeFor('in_memory'), 'in_memory')
  })

  it('writes the schema enum and preserves unknown adapter config', () => {
    const original = { file_path: '/tmp/state', extension: { enabled: true } }

    assert.deepEqual(withPersistenceMode(original, 'file_based'), {
      file_path: '/tmp/state',
      extension: { enabled: true },
      store_method: 'file_based',
    })
    assert.deepEqual(withPersistenceMode(original, 'in_memory'), {
      extension: { enabled: true },
      store_method: 'in_memory',
    })
    assert.deepEqual(original, {
      file_path: '/tmp/state',
      extension: { enabled: true },
    })
  })
})
