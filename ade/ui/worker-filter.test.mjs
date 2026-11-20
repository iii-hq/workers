import assert from 'node:assert/strict'
import test from 'node:test'
import esbuild from 'esbuild'

// The Observers page filters by worker name, and the names come from the
// catalogue, never from a list in the interface. A worker is involved with
// a function group when it hosts the function or registered an observer
// under it.
async function loadWorkerFilter() {
  const result = await esbuild.build({
    entryPoints: ['src/catalog/worker-filter.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

const observer = (id, worker_name) => ({ id, worker: { worker_id: `${id}-w`, worker_name } })
const groups = [
  {
    source: { function_id: 'http::on-request', worker_name: 'http' },
    observers: [observer('a', 'shop'), observer('b', 'billing')],
  },
  {
    source: { function_id: 'cron::on-tick', worker_name: 'cron' },
    observers: [observer('c', 'shop')],
  },
  { source: { function_id: 'shop::on-order', worker_name: 'shop' }, observers: [] },
  { source: { function_id: 'db::on-row', worker_name: 'database' }, observers: [observer('d', undefined)] },
]

test('every worker the catalogue names is offered, counted by group', async () => {
  const { workerCounts } = await loadWorkerFilter()
  assert.deepEqual(
    [...workerCounts(groups).entries()].sort(),
    [
      ['billing', 1],
      ['cron', 1],
      ['database', 1],
      ['http', 1],
      ['shop', 3],
    ],
  )
})

test('no worker keeps every group as it is', async () => {
  const { narrowToWorker } = await loadWorkerFilter()
  assert.deepEqual(narrowToWorker(groups, null), groups)
})

test('a worker keeps the functions it hosts whole and narrows the others to its observers', async () => {
  const { narrowToWorker } = await loadWorkerFilter()
  const shop = narrowToWorker(groups, 'shop')
  assert.deepEqual(
    shop.map((g) => [g.source.function_id, g.observers.map((o) => o.id)]),
    [
      ['http::on-request', ['a']],
      ['cron::on-tick', ['c']],
      ['shop::on-order', []],
    ],
  )
  // The host keeps every observer, whoever registered it.
  assert.deepEqual(narrowToWorker(groups, 'http').map((g) => g.observers.map((o) => o.id)), [['a', 'b']])
})

test('a worker nobody names matches nothing', async () => {
  const { narrowToWorker } = await loadWorkerFilter()
  assert.deepEqual(narrowToWorker(groups, 'ghost'), [])
})
