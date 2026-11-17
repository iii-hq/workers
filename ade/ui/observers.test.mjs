import assert from 'node:assert/strict'
import test from 'node:test'
import esbuild from 'esbuild'

// The Observers screen reads `engine::observers::list` rows. Engines with
// `observe.capture` report which outcomes fire the target as
// `fires_on: ["success" | "error", ...]`; `fires` is always the numeric
// activation count. Older engines (0.25 and the 0.26 rc) report the legacy
// `on` / `deliver` instead. The screen must read both without deriving
// firing from `capture`.
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

// A legacy engine's listing still reports the raw delivery it used; this is
// a listing row, not an observe source.
const LEGACY_RAW_DELIVERY = 'output'

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
  const info = observerInfo({ ...base, capture: ['input', 'error'], fires_on: ['error'], fires: 3 })
  assert.deepEqual(info.outcomes, ['error'])
  assert.equal(info.fires, 3, 'numeric fires stays the activation count')
  const facts = observerDeliveryFacts(info)
  assert.equal(fact(facts, 'Fires on'), 'error')
  assert.equal(fact(facts, 'On'), undefined, 'the legacy `on: success` row is gone')
  assert.equal(fact(facts, 'Deliver'), undefined, 'no deliver row for an envelope observer')
  assert.equal(fact(facts, 'Capture'), 'input, error')
})

test('fires_on lists every outcome in order', async () => {
  const { observerInfo, observerDeliveryFacts } = await loadObserverInfo()
  const info = observerInfo({ ...base, capture: ['input'], fires_on: ['error', 'success'] })
  assert.deepEqual(info.outcomes, ['success', 'error'])
  assert.equal(fact(observerDeliveryFacts(info), 'Fires on'), 'success, error')
})

test('firing is never derived from capture', async () => {
  const { observerInfo } = await loadObserverInfo()
  const info = observerInfo({ ...base, capture: ['input', 'error'], fires_on: ['success'] })
  assert.deepEqual(info.outcomes, ['success'])
})

test('fires_on wins over a legacy on', async () => {
  const { observerInfo } = await loadObserverInfo()
  assert.deepEqual(observerInfo({ ...base, fires_on: ['success'], on: 'error' }).outcomes, [
    'success',
  ])
})

test('unknown entries in fires_on are ignored; an unusable fires_on falls back to on', async () => {
  const { observerInfo } = await loadObserverInfo()
  assert.deepEqual(observerInfo({ ...base, fires_on: ['error', 'bogus', 3] }).outcomes, ['error'])
  assert.deepEqual(observerInfo({ ...base, fires_on: [], on: 'error' }).outcomes, ['error'])
  assert.deepEqual(observerInfo({ ...base, fires_on: 'error', on: 'complete' }).outcomes, [
    'success',
    'error',
  ])
})

test('an outcome array under fires is not read as fires_on', async () => {
  const { observerInfo } = await loadObserverInfo()
  const info = observerInfo({ ...base, fires: ['error'] })
  assert.deepEqual(info.outcomes, ['success'], 'fires is the count, never the outcome list')
  assert.equal(info.fires, 0)
})

test('a legacy engine row: on error, deliver output, numeric fires count', async () => {
  const { observerInfo, observerDeliveryFacts } = await loadObserverInfo()
  const info = observerInfo({ ...base, on: 'error', deliver: LEGACY_RAW_DELIVERY, fires: 7, capture: [] })
  assert.deepEqual(info.outcomes, ['error'])
  assert.equal(info.fires, 7)
  assert.equal(info.deliver, 'output')
  const facts = observerDeliveryFacts(info)
  assert.equal(fact(facts, 'Fires on'), 'error')
  assert.equal(fact(facts, 'Deliver'), 'output', 'a legacy raw delivery is still shown')
})

test('legacy on: absent means success, complete means both', async () => {
  const { observerInfo } = await loadObserverInfo()
  assert.deepEqual(observerInfo({ ...base }).outcomes, ['success'])
  assert.deepEqual(observerInfo({ ...base, on: 'complete' }).outcomes, ['success', 'error'])
})

test('the activation count sits beside fires_on', async () => {
  const { observerInfo } = await loadObserverInfo()
  const info = observerInfo({ ...base, fires_on: ['success'], fires: 12 })
  assert.equal(info.fires, 12)
  assert.deepEqual(info.outcomes, ['success'])
})

// QA 2026-11-16 (F11): the Observers page read "refresh", "copy id",
// "dispose", "reset", "run now" and "copy iii command" next to "Run Now" and
// the rest of the console's sentence case. Every label a person reads on the
// catalog pages starts with a capital; ids, values and code stay as they are.
test('catalog labels are sentence case', async () => {
  const { readFileSync, readdirSync } = await import('node:fs')
  const dir = new URL('./src/catalog/', import.meta.url)
  const offenders = []
  for (const name of readdirSync(dir).filter((file) => file.endsWith('.tsx'))) {
    const lines = readFileSync(new URL(name, dir), 'utf8').split('\n')
    lines.forEach((line, index) => {
      const at = `${name}:${index + 1}: ${line.trim()}`
      // Props a person reads.
      if (/\b(?<![-\w])(label|runningLabel|title|placeholder|headline|eyebrow|body)=["'][a-z]/.test(line)) offenders.push(at)
      if (/(?<![-\w])(label|runningLabel|title|placeholder)=\{[^}]*\? '[a-z][^']*' : '[a-z]/.test(line)) offenders.push(at)
      // A control's words chosen by state: {busy ? 'loading…' : 'refresh'}.
      if (/^\s+\{[\w.]+ \? '[a-z][^']*' : '[a-z][^']*'\}$/.test(line)) offenders.push(at)
      // A button's or tab's own words on a line of their own.
      if (/^\s+[a-z][a-z ]+$/.test(line) && /<(Button|TabsTrigger)\b[^>]*>?\s*$|^\s+>\s*$|^\s+onClick=|disabled=/.test(lines[index - 1] ?? '')) offenders.push(at)
    })
  }
  assert.deepEqual(offenders, [])
})
