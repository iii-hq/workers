import assert from 'node:assert/strict'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { observeUiAsset, registerUiContent, uiAssets, UI_CONTENT } from '../src/ui-assets.mjs'

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/** An `iii` that records what the worker registers and observes. */
const stubIii = () => {
  const functions = new Map()
  const observed = []
  return {
    functions,
    observed,
    registerFunction(id, handler, options) {
      functions.set(id, { handler, options })
      return { id, unregister() {} }
    },
    observe(source, target, options) {
      const listeners = new Set()
      let settle
      const ready = new Promise((resolve, reject) => {
        settle = { resolve, reject }
      })
      const handle = {
        ready: () => ready,
        onStateChange(listener) {
          listeners.add(listener)
          return () => listeners.delete(listener)
        },
        async dispose() {},
        emit: (state, reason) => [...listeners].forEach((listener) => listener(state, reason)),
        settle,
      }
      observed.push({ source, target, options, handle })
      return handle
    },
  }
}

test('learn-iii::ui-content answers { content, content_type }', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'learn-iii-ui-'))
  writeFileSync(join(dir, 'page.js'), 'export default () => {}')
  writeFileSync(join(dir, 'styles.css'), '.learn {}')
  const iii = stubIii()
  registerUiContent(iii, dir)

  const { handler } = iii.functions.get(UI_CONTENT)
  assert.deepEqual(await handler({ path: 'learn-iii/page.js' }), {
    content: 'export default () => {}',
    content_type: 'text/javascript',
  })
  assert.deepEqual(await handler({ path: 'learn-iii/styles.css' }), { content: '.learn {}', content_type: 'text/css' })
  await assert.rejects(handler({ path: 'learn-iii/other.js' }), /unknown ui asset/)
})

test('each asset is observed on ade::ui::asset with { kind, path }, targeting the content function', () => {
  const iii = stubIii()
  for (const path of Object.keys(uiAssets)) observeUiAsset(iii, path, { log: () => {} })
  assert.deepEqual(
    iii.observed.map(({ source, target }) => ({ source, target })),
    [
      {
        source: { function_id: 'ade::ui::asset', args: { kind: 'script', path: 'learn-iii/page.js' } },
        target: 'learn-iii::ui-content',
      },
      {
        source: { function_id: 'ade::ui::asset', args: { kind: 'style', path: 'learn-iii/styles.css' } },
        target: 'learn-iii::ui-content',
      },
    ],
  )
  for (const { handle } of iii.observed) handle.settle.resolve()
})

test('refusals and engine disposals are logged; supersede and our own dispose are not', async () => {
  const iii = stubIii()
  const logs = []
  observeUiAsset(iii, 'learn-iii/page.js', { log: (line) => logs.push(line) })
  const { handle } = iii.observed[0]
  handle.emit('error', { code: 'UI_ASSET_SUPERSEDED', message: 'newer observer' })
  handle.emit('disposed', { code: 'UNOBSERVED', message: 'disposed by the observing worker' })
  assert.deepEqual(logs, [])
  handle.emit('error', { code: 'FORBIDDEN', message: 'not allowed' })
  handle.emit('disposed', { code: 'OBSERVER_REPLACED', message: 'reattached' })
  assert.equal(logs.length, 2)
  assert.match(logs[0], /learn-iii\/page\.js rejected: FORBIDDEN not allowed/)
  assert.match(logs[1], /learn-iii\/page\.js observer disposed: OBSERVER_REPLACED reattached/)
  handle.settle.reject(new Error('gone'))
})

test('an asset observer still pending after the deadline is warned about, once', async () => {
  const iii = stubIii()
  const logs = []
  observeUiAsset(iii, 'learn-iii/page.js', { warnAfterMs: 10, log: (line) => logs.push(line) })
  observeUiAsset(iii, 'learn-iii/styles.css', { warnAfterMs: 10, log: (line) => logs.push(line) })
  iii.observed[1].handle.settle.resolve()
  await wait(40)
  assert.equal(logs.length, 1)
  assert.match(logs[0], /learn-iii\/page\.js is still not served after 10 ms/)
  assert.match(logs[0], /ade::ui::asset/)
})
