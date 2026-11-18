import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  createEntry,
  deleteEntry,
  deleteScope,
  KEY_EXISTS,
  type StateCaller,
  validateNewEntry,
} from '../src/page/entries.ts'

/** An in-memory state worker that records every call. */
function fakeState(initial: Record<string, Record<string, unknown>> = {}) {
  const store = new Map(Object.entries(initial).map(([s, kv]) => [s, new Map(Object.entries(kv))]))
  const calls: Array<{ id: string; data: any }> = []
  const iii: StateCaller = {
    async call<T>(id: string, data?: any): Promise<T> {
      calls.push({ id, data })
      const scope = store.get(data.scope) ?? new Map()
      switch (id) {
        case 'state::compare-and-set': {
          if (scope.has(data.key)) return { swapped: false, current: scope.get(data.key) } as T
          scope.set(data.key, data.value)
          store.set(data.scope, scope)
          return { swapped: true } as T
        }
        case 'state::list_keys':
          return { keys: [...scope.keys()] } as T
        case 'state::delete': {
          const old = scope.get(data.key) ?? null
          scope.delete(data.key)
          if (scope.size === 0) store.delete(data.scope)
          return old as T
        }
        default:
          throw new Error(`unexpected ${id}`)
      }
    },
  }
  return { iii, calls, store }
}

describe('new state entry', () => {
  it('needs a scope, a key and valid JSON', () => {
    assert.deepEqual(validateNewEntry({ scope: 'qa', key: 'k', value: '{}' }), {})
    const errors = validateNewEntry({ scope: ' ', key: '', value: '{' })
    assert.equal(errors.scope, 'Name the scope.')
    assert.equal(errors.key, 'Name the key.')
    assert.match(errors.value ?? '', /^Not valid JSON/)
  })

  it('creates a key in a new scope with the parsed value, trimmed names', async () => {
    const { iii, calls, store } = fakeState()
    const created = await createEntry(iii, { scope: ' fresh ', key: ' first ', value: '{"a":1}' })
    assert.deepEqual(created, { scope: 'fresh', key: 'first' })
    assert.deepEqual(calls, [
      { id: 'state::compare-and-set', data: { scope: 'fresh', key: 'first', value: { a: 1 } } },
    ])
    assert.deepEqual(store.get('fresh')?.get('first'), { a: 1 })
  })

  it('never overwrites a key that already exists', async () => {
    const { iii, store } = fakeState({ qa: { k: 'kept' } })
    await assert.rejects(createEntry(iii, { scope: 'qa', key: 'k', value: '"new"' }), {
      message: KEY_EXISTS,
    })
    assert.equal(store.get('qa')?.get('k'), 'kept')
  })

  it('does not call the worker for an invalid draft', async () => {
    const { iii, calls } = fakeState()
    await assert.rejects(createEntry(iii, { scope: 'qa', key: 'k', value: 'nope' }))
    assert.equal(calls.length, 0)
  })
})

describe('deleting state', () => {
  it('deletes one key', async () => {
    const { iii, store } = fakeState({ qa: { a: 1, b: 2 } })
    await deleteEntry(iii, 'qa', 'a')
    assert.deepEqual([...(store.get('qa')?.keys() ?? [])], ['b'])
  })

  it('deletes a scope by deleting every key the worker lists now', async () => {
    const { iii, calls, store } = fakeState({ qa: { a: 1, b: 2, c: 3 }, other: { x: 1 } })
    assert.equal(await deleteScope(iii, 'qa'), 3)
    assert.equal(store.has('qa'), false)
    assert.equal(store.has('other'), true)
    assert.deepEqual(
      calls.map((c) => c.id),
      ['state::list_keys', 'state::delete', 'state::delete', 'state::delete'],
    )
  })
})
