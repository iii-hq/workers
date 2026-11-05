import assert from 'node:assert/strict'
import test from 'node:test'
import esbuild from 'esbuild'

// The Observers screen reads `engine::observers::list` rows. Engines with
// `observe.capture` report which outcomes fire the target as
// `fires: ["success" | "error", ...]`; older engines (0.25 and the 0.26 rc)
// report the legacy `on` / `deliver` instead, and `fires` as the activation
// count. The screen must read both without deriving firing from `capture`.
async function loadObserverInfo() {
  const result = await esbuild.build({
    entryPoints: ['src/catalog/observer-info.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

const base = {
  id: 'obs-1',
  function_id: 'payments::charge',
  namespace: 'default',
  target: { function_id: 'alerts::payment-failed', namespace: 'default' },
  state: 'active',
}

function fact(facts, label) {
  return facts.find((f) => f.label === label)?.value
}

test('an error-only observer from an observe.capture engine fires on error', async () => {
  const { observerInfo, observerDeliveryFacts } = await loadObserverInfo()
  const info = observerInfo({ ...base, capture: ['input', 'error'], fires: ['error'] })
  assert.deepEqual(info.outcomes, ['error'])
  const facts = observerDeliveryFacts(info)
  assert.equal(fact(facts, 'fires on'), 'error')
  assert.equal(fact(facts, 'on'), undefined, 'the legacy `on: success` row is gone')
  assert.equal(fact(facts, 'deliver'), undefined, 'no deliver row for an envelope observer')
  assert.equal(fact(facts, 'capture'), 'input, error')
})

test('fires lists every outcome in order', async () => {
  const { observerInfo, observerDeliveryFacts } = await loadObserverInfo()
  const info = observerInfo({ ...base, capture: ['input'], fires: ['success', 'error'] })
  assert.deepEqual(info.outcomes, ['success', 'error'])
  assert.equal(fact(observerDeliveryFacts(info), 'fires on'), 'success, error')
})

test('firing is never derived from capture', async () => {
  const { observerInfo } = await loadObserverInfo()
  const info = observerInfo({ ...base, capture: ['input', 'error'], fires: ['success'] })
  assert.deepEqual(info.outcomes, ['success'])
})

test('unknown entries in fires are ignored; an unusable fires falls back to on', async () => {
  const { observerInfo } = await loadObserverInfo()
  assert.deepEqual(observerInfo({ ...base, fires: ['error', 'bogus', 3] }).outcomes, ['error'])
  assert.deepEqual(observerInfo({ ...base, fires: [], on: 'error' }).outcomes, ['error'])
})

test('a legacy engine row: on error, deliver output, numeric fires count', async () => {
  const { observerInfo, observerDeliveryFacts } = await loadObserverInfo()
  const info = observerInfo({ ...base, on: 'error', deliver: 'output', fires: 7, capture: [] })
  assert.deepEqual(info.outcomes, ['error'])
  assert.equal(info.fires, 7)
  assert.equal(info.deliver, 'output')
  const facts = observerDeliveryFacts(info)
  assert.equal(fact(facts, 'fires on'), 'error')
  assert.equal(fact(facts, 'deliver'), 'output', 'a legacy raw delivery is still shown')
})

test('legacy on: absent means success, complete means both', async () => {
  const { observerInfo } = await loadObserverInfo()
  assert.deepEqual(observerInfo({ ...base }).outcomes, ['success'])
  assert.deepEqual(observerInfo({ ...base, on: 'complete' }).outcomes, ['success', 'error'])
})

test('the activation count survives a fires outcome array', async () => {
  const { observerInfo } = await loadObserverInfo()
  const info = observerInfo({ ...base, fires: ['success'] })
  assert.equal(info.fires, 0)
  assert.deepEqual(info.outcomes, ['success'])
})
