import assert from 'node:assert/strict'
import { test } from 'node:test'
import { capNewest, execBody, type Msg, newState, resultText, SUMMARY_PREFIX, step } from './core.ts'

const user = (text: string): Msg => ({ role: 'user', content: [{ type: 'text', text }], timestamp: 1 })
const call = (id: string): Msg => ({
  role: 'assistant',
  content: [{ type: 'function_call', id, function_id: 'shell::exec', arguments: { command: 'ls' } }],
  stop_reason: 'function_call',
  model: 'm',
  provider: 'p',
  timestamp: 1,
})
const result = (id: string, stdout: string): Msg => ({
  role: 'function_result',
  function_call_id: id,
  content: [{ type: 'text', text: JSON.stringify({ exit_code: 0, stdout, stderr: '' }) }],
  timestamp: 1,
})
const reply = (text: string): Msg => ({
  role: 'assistant',
  content: [{ type: 'text', text }],
  stop_reason: 'end',
  model: 'm',
  provider: 'p',
  timestamp: 1,
})
const noSum = async () => ({ text: '', cost: 0 })

test('execBody renders output and exit code, and keeps head and tail of long output', () => {
  assert.equal(execBody(JSON.stringify({ exit_code: 2, stdout: 'a', stderr: 'b' })), 'a\nb\n\n(exit_code=2)')
  assert.equal(execBody(JSON.stringify({ exit_code: 0, stdout: '', stderr: '' })), '(no output)\n\n(exit_code=0)')
  assert.equal(execBody(JSON.stringify({ exit_code: null, timed_out: true, stdout: 'x' })), 'x\n\n(exit_code=124)')
  const long = execBody(JSON.stringify({ exit_code: 0, stdout: 'h'.repeat(6000) + 't'.repeat(6000) }))
  assert.ok(long.includes('h'.repeat(5000)) && long.includes('t'.repeat(5000)) && !long.includes('h'.repeat(5001)))
  assert.equal(execBody('plain text'), 'plain text')
})

test('capNewest keeps text that fits and cuts the middle of text that does not', () => {
  assert.equal(capNewest('abc', 10, 65536), 'abc')
  const cut = capNewest('x'.repeat(4000), 200, 65536)
  assert.ok(cut.length < 1000 && cut.startsWith('x'.repeat(304)) && cut.endsWith('x'.repeat(304)))
  assert.match(cut, /3392 chars cut to stay inside the 65536-token context limit/)
})

test('base: readout on the newest result, one final turn over the limit, then stop', async () => {
  const s = newState()
  const cfg = { arm: 'base', budget: 4096 } as const
  const raw = [user('task'), call('c1'), result('c1', 'x'.repeat(400))]
  const r1 = await step(s, { messages: raw, system: '' }, cfg, 't1:0', noSum)
  assert.equal(r1.stop, false)
  assert.match(resultText(r1.messages[2]), /\(exit_code=0\)\n\[context: ~\d+\/2048 tokens\]$/)
  raw.push(user('y'.repeat(9000)))
  const r2 = await step(s, { messages: raw, system: '' }, cfg, 't2:0', noSum)
  assert.equal(r2.stop, false)
  assert.match(resultText(r2.messages.at(-1)!), /This is your final turn/)
  raw.push(call('c2'), result('c2', 'z'))
  const r3 = await step(s, { messages: raw, system: '' }, cfg, 't2:1', noSum)
  assert.equal(r3.stop, true)
  assert.match(resultText(r3.messages.at(-1)!), /over the limit\]$/)
  assert.deepEqual([s.stats.steps, s.stats.stops], [3, 1])
})

test('sum: summarises at 75% of the budget into one handoff message; a failed summary keeps the history', async () => {
  const cfg = { arm: 'sum', budget: 16384 } as const
  const raw = [user('task'), user('a'.repeat(50_000)), reply('ok')]
  const s = newState()
  let seen = ''
  const r = await step(s, { messages: raw, system: '' }, cfg, 't:0', async (t) => {
    seen = t
    return { text: 'SUMMARY', cost: 0.5 }
  })
  assert.ok(seen.startsWith('[[user]]\ntask') && seen.includes('a'.repeat(100)))
  assert.deepEqual(r.messages.map(resultText), ['task', `${SUMMARY_PREFIX}\nSUMMARY`])
  assert.deepEqual([s.stats.summaries, s.stats.summary_cost_usd], [1, 0.5])
  const s2 = newState()
  const r2 = await step(s2, { messages: raw, system: '' }, cfg, 't:0', noSum)
  assert.deepEqual(r2.messages.map(resultText), ['task', 'a'.repeat(50_000), 'ok'])
  assert.equal(s2.stats.summary_failures, 1)
})

test('the same (turn, step) returns the cached request without a new step', async () => {
  const s = newState()
  const cfg = { arm: 'base', budget: 65536 } as const
  const raw = [user('task'), call('c1'), result('c1', 'out')]
  const a = await step(s, { messages: raw, system: '' }, cfg, 't:1', noSum)
  const b = await step(s, { messages: raw, system: '' }, cfg, 't:1', noSum)
  assert.equal(b.messages, a.messages)
  assert.equal(s.stats.steps, 1)
})

test('a rewritten history rebuilds the projection from the window', async () => {
  const s = newState()
  const cfg = { arm: 'base', budget: 65536 } as const
  await step(s, { messages: [user('task'), user('one')], system: '' }, cfg, 't:0', noSum)
  const r = await step(s, { messages: [user('task'), user('two')], system: '' }, cfg, 't:1', noSum)
  assert.deepEqual(r.messages.map(resultText), ['task', 'two'])
})

test('with a calibration ratio above 1 the newest result is cut to fit the calibrated limit, not given a final turn', async () => {
  const s = newState()
  const cfg = { arm: 'base', budget: 16384 } as const
  const raw: Msg[] = [user(`task ${'a'.repeat(34_000)}`), call('c1'), result('c1', 'out')]
  await step(s, { messages: raw, system: '' }, cfg, 't:0', noSum)
  raw.push({ ...call('c2'), usage: { input: Math.round(1.5 * s.rawSent) } }, result('c2', 'z'.repeat(9000)))
  const r = await step(s, { messages: raw, system: '' }, cfg, 't:1', noSum)
  assert.equal(s.ratio.toFixed(2), '1.50')
  assert.match(resultText(r.messages.at(-1)!), /chars cut to stay inside the 14336-token context limit/)
  assert.ok(r.tc <= 16384 - 2048, `tc ${r.tc}`)
  assert.equal(s.finalized, false)
})

test('a new turn after a stop gets its own final turn, and a replay reports that step, not the peak', async () => {
  const s = newState()
  const cfg = { arm: 'base', budget: 4096 } as const
  const raw = [user('task'), user('y'.repeat(9000))]
  const r1 = await step(s, { messages: raw, system: '' }, cfg, 't1:0', noSum)
  assert.match(resultText(r1.messages.at(-1)!), /This is your final turn/)
  raw.push(call('c1'), result('c1', 'z'))
  const r2 = await step(s, { messages: raw, system: '' }, cfg, 't1:1', noSum)
  assert.equal(r2.stop, true)
  const again = await step(s, { messages: raw, system: '' }, cfg, 't1:1', noSum)
  assert.deepEqual([again.stop, again.tc], [true, r2.tc])
  raw.push(user('next question'))
  const r3 = await step(s, { messages: raw, system: '' }, cfg, 't2:0', noSum)
  assert.equal(r3.stop, false)
  assert.match(resultText(r3.messages.at(-1)!), /This is your final turn/)
})

test('sum: a summary that stays above 75% is not redone every step', async () => {
  const s = newState()
  const cfg = { arm: 'sum', budget: 16384 } as const
  let calls = 0
  const big = async () => {
    calls++
    return { text: 'S'.repeat(52_000), cost: 0 }
  }
  const raw = [user('task'), user('a'.repeat(50_000)), reply('ok')]
  await step(s, { messages: raw, system: '' }, cfg, 't:0', big)
  for (let i = 1; i <= 3; i++) {
    raw.push(user(`more ${i}`))
    await step(s, { messages: raw, system: '' }, cfg, `t:${i}`, big)
  }
  assert.equal(calls, 1)
})

test('sum: user messages the model has not seen yet stay out of the summary and follow it', async () => {
  const cfg = { arm: 'sum', budget: 16384 } as const
  let seen = ''
  const sum = async (t: string) => {
    seen = t
    return { text: 'SUMMARY', cost: 0 }
  }
  const raw = [user('task'), user('a'.repeat(50_000)), reply('ok'), user('NEW OP')]
  const r = await step(newState(), { messages: raw, system: '' }, cfg, 't:0', sum)
  assert.ok(seen.includes('a'.repeat(100)) && !seen.includes('NEW OP'), seen.slice(-200))
  assert.deepEqual(r.messages.map(resultText), ['task', `${SUMMARY_PREFIX}\nSUMMARY`, 'NEW OP'])
  // Nothing seen yet: there is nothing to summarise, so the new message goes out whole.
  seen = ''
  const unseen = [user('task'), user('a'.repeat(50_000))]
  const r2 = await step(newState(), { messages: unseen, system: '' }, cfg, 't:0', sum)
  assert.equal(seen, '')
  assert.deepEqual(r2.messages.map(resultText), ['task', 'a'.repeat(50_000)])
})

test('sum: a summary whose cost is unknown (timeout or error) is counted, not priced at $0', async () => {
  const s = newState()
  const cfg = { arm: 'sum', budget: 16384 } as const
  const raw = [user('task'), user('a'.repeat(50_000)), reply('ok')]
  await step(s, { messages: raw, system: '' }, cfg, 't:0', async () => ({ text: '', cost: null }))
  assert.deepEqual([s.stats.summary_failures, s.stats.summary_cost_unknown, s.stats.summary_cost_usd], [1, 1, 0])
})
