import assert from 'node:assert/strict'
import test from 'node:test'
import { createRefreshGate } from './refresh-gate.js'

function deferred() {
  let resolve
  const promise = new Promise((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test('coalesces repeated in-flight refreshes into one follow-up', async () => {
  const releases = [deferred(), deferred()]
  let calls = 0
  const gate = createRefreshGate(async () => {
    const release = releases[calls]
    calls += 1
    await release.promise
  })

  void gate.request()
  void gate.request()
  void gate.request()
  await Promise.resolve()
  assert.equal(calls, 1)

  releases[0].resolve()
  await Promise.resolve()
  await Promise.resolve()
  assert.equal(calls, 2)

  releases[1].resolve()
  await gate.whenIdle()
  assert.equal(calls, 2)
})
