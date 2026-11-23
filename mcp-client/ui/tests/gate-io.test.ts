import type { ExtensionIii } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { gateDeps, probeVerdicts, readGate, writeBlock } from '../src/gate-io'
import { A, L, rule } from './fixtures/gate'

interface Opts {
  idError?: unknown
  afterSet?: unknown
  flight?: { now: number; max: number }
  failFor?: string
}

/** The calls the gate I/O makes, answered from one stored entry. */
function fakeIii(entry: unknown, opts: Opts = {}) {
  let stored = entry
  const calls: { fn: string; payload: Record<string, unknown> }[] = []
  const call = async (fn: string, payload: Record<string, unknown>) => {
    calls.push({ fn, payload })
    if (fn === 'approval-gate::configuration-id') {
      if (opts.idError) throw opts.idError
      return { id: 'approval-gate' }
    }
    if (fn === 'configuration::get') return { value: stored }
    if (fn === 'configuration::set') {
      // `afterSet`: somebody else replaced the entry right after this write.
      stored = 'afterSet' in opts ? opts.afterSet : payload.value
      return {}
    }
    if (fn === 'approval::evaluate') {
      const { flight } = opts
      if (flight) {
        flight.now += 1
        flight.max = Math.max(flight.max, flight.now)
      }
      await new Promise((resolve) => setTimeout(resolve, 0))
      if (flight) flight.now -= 1
      if (payload.function_id === opts.failFor) throw new Error('evaluate failed')
      return { verdict: 'needs_approval', reason: 'held' }
    }
    throw new Error(`unexpected call ${fn}`)
  }
  return { iii: { call } as unknown as ExtensionIii, calls }
}

const NO_GATE = { idError: { code: 'function_not_found', message: 'x' } }
const ENTRY = {
  default_mode: 'manual',
  grant_reask_limit: 3,
  rules: [
    '!approval::*',
    // biome-ignore lint/suspicious/noTemplateCurlyInString: the gate's ${VAR} text, kept as written
    { function: 'web::fetch', action: 'allow', args: { url: { matches: '${ALLOWED_HOST}' } } },
    rule(A('whoami')),
  ],
}
const fns = (calls: { fn: string }[]) => calls.map((c) => c.fn)

describe('readGate', () => {
  it('reads the gate entry raw', async () => {
    const f = fakeIii(ENTRY)
    expect(await readGate(f.iii)).toEqual({ installed: true, id: 'approval-gate', value: ENTRY, rules: ENTRY.rules })
    expect(f.calls[1]).toEqual({ fn: 'configuration::get', payload: { id: 'approval-gate', raw: true } })
  })

  it('reads "function not found" from the error code, never from the text', async () => {
    const missing = fakeIii(ENTRY, { idError: { code: 'function_not_found', message: 'no handler' } })
    expect(await readGate(missing.iii)).toEqual({ installed: false, id: null, value: null, rules: null })
    const slow = fakeIii(ENTRY, { idError: { code: 'timeout', message: 'function not found in time' } })
    expect(await readGate(slow.iii)).toMatchObject({ installed: true, rules: null, error: expect.any(String) })
  })

  it('reports an entry with no rules array as unreadable', async () => {
    expect(await readGate(fakeIii({ default_mode: 'manual' }).iii)).toMatchObject({
      installed: true,
      rules: null,
      error: 'The approval-gate entry has no rules array.',
    })
  })
})

describe('writeBlock', () => {
  it('re-reads, writes once with the keys it read, and re-reads', async () => {
    const f = fakeIii(ENTRY)
    const out = await writeBlock(f.iii, 'linear', [L('whoami'), L('get_issue')])
    expect(fns(f.calls)).toEqual([
      'approval-gate::configuration-id',
      'configuration::get',
      'configuration::set',
      'configuration::get',
    ])
    const rules = [...ENTRY.rules, rule(L('get_issue')), rule(L('whoami'))]
    expect(f.calls[2].payload).toEqual({
      id: 'approval-gate',
      value: { default_mode: 'manual', grant_reask_limit: 3, rules },
    })
    expect(out).toMatchObject({ ok: true, written: 2 })
    expect(out.stored).toEqual(rules)
  })

  it('says so and does not retry when the stored block is not the written one', async () => {
    const f = fakeIii(ENTRY, { afterSet: ENTRY })
    expect(await writeBlock(f.iii, 'linear', [L('whoami')])).toEqual({
      ok: false,
      reason: 'The approval gate did not keep the rules as written. Its stored rules are shown.',
      written: 0,
      stored: ENTRY.rules,
    })
    expect(fns(f.calls).filter((fn) => fn === 'configuration::set')).toHaveLength(1)
  })

  it('writes nothing to an entry it cannot merge', async () => {
    const f = fakeIii({ rules: 'x' })
    expect(await writeBlock(f.iii, 'linear', [L('whoami')])).toEqual({
      ok: false,
      reason: 'The approval-gate entry has no rules array.',
      written: 0,
      stored: null,
    })
    expect(fns(f.calls)).not.toContain('configuration::set')
  })

  it('writes nothing when there is no gate', async () => {
    const f = fakeIii(ENTRY, NO_GATE)
    expect(await writeBlock(f.iii, 'linear', [L('whoami')])).toMatchObject({ ok: false, written: 0 })
    expect(fns(f.calls)).toEqual(['approval-gate::configuration-id'])
  })
})

describe('gateDeps', () => {
  it('counts and removes a block', async () => {
    const deps = gateDeps(fakeIii({ rules: [rule(L('whoami')), rule(L('get_issue')), rule(A('whoami'))] }).iii)
    expect(await deps.blockCount('linear')).toBe(2)
    expect(await deps.removeBlock('linear')).toEqual({ ok: true, count: 2 })
    expect(await deps.blockCount('linear')).toBe(0)
    expect(await deps.blockCount('linear-app')).toBe(1)
  })

  it('counts zero with no gate, and refuses to guess when the gate cannot be read', async () => {
    expect(await gateDeps(fakeIii(ENTRY, NO_GATE).iii).blockCount('linear')).toBe(0)
    await expect(gateDeps(fakeIii({ rules: 'x' }).iii).blockCount('linear')).rejects.toThrow(
      'The approval-gate entry has no rules array.',
    )
  })
})

describe('probeVerdicts', () => {
  it('asks the gate eight at a time, under the probe session', async () => {
    const flight = { now: 0, max: 0 }
    const ids = Array.from({ length: 20 }, (_, i) => L(`tool_${i}`))
    const f = fakeIii(ENTRY, { flight, failFor: L('tool_7') })
    const out = await probeVerdicts(f.iii, ids)
    expect(flight.max).toBe(8)
    expect(
      f.calls.every((c) => c.fn === 'approval::evaluate' && c.payload.session_id === 'mcp-client-page-probe'),
    ).toBe(true)
    expect(f.calls.map((c) => String(c.payload.function_id)).sort()).toEqual([...ids].sort())
    expect(Object.keys(out)).toHaveLength(19)
    expect(out[L('tool_0')]).toEqual({ verdict: 'needs_approval', reason: 'held' })
    // A failed probe is no verdict, never "allow".
    expect(out).not.toHaveProperty([L('tool_7')])
  })
})
