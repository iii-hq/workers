import { describe, expect, it } from 'vitest'
import {
  blockOf,
  gateStatus,
  mergeGateEntry,
  mergeGateRules,
  orphanBlocks,
  ruleFor,
  type Verdict,
} from '../src/gate-rules'
import type { Server } from '../src/types'
import { A, fa, L, rule } from './fixtures/gate'
import schema from './fixtures/gate-rule.schema.json'
import { server as srv } from './fixtures/servers'

const access = [
  fa('get_issue', 'read', 'name'),
  fa('list_issues', 'read', 'server_read'),
  fa('save_issue', 'write', 'server_write'),
  fa('delete_issue', 'destructive', 'server_write'),
]
const present = [rule(L('get_issue')), rule(L('list_issues'))]
const st = (
  rules: unknown[] | null,
  over: Partial<Server> = {},
  verdicts?: Record<string, Verdict>,
  installed = true,
) => gateStatus({ gate: { installed, rules }, server: srv(over), access, verdicts })

describe('ruleFor', () => {
  it('writes the rule of spec 5.6', () => {
    expect(ruleFor(L('list_issues'))).toEqual({
      function: 'mcp::linear::list_issues',
      action: 'allow',
      rule_id: 'mcp-client:mcp::linear::list_issues',
    })
  })

  it('writes a rule the gate schema accepts', () => {
    const object = schema.oneOf.find((branch) => branch.type === 'object') as (typeof schema.oneOf)[1]
    const written = ruleFor(L('list_issues'))
    expect(object.additionalProperties).toBe(false)
    expect(Object.keys(object.properties)).toEqual(expect.arrayContaining(Object.keys(written)))
    expect(Object.keys(written)).toEqual(expect.arrayContaining(object.required))
    expect(object.properties.action.enum).toContain(written.action)
    for (const value of Object.values(written)) expect(typeof value).toBe('string')
    expect(written).not.toHaveProperty('modes')
  })
})

describe('mergeGateRules', () => {
  const m = (rules: unknown[], ids: string[]) => mergeGateRules(rules, 'linear', ids)

  it('puts the block at the end of an empty or shipped array, sorted by id', () => {
    expect(m([], [L('whoami'), L('get_issue')])).toEqual([rule(L('get_issue')), rule(L('whoami'))])
    expect(m(['!approval::*'], [L('whoami')])).toEqual(['!approval::*', rule(L('whoami'))])
  })

  it('keeps every foreign item as the same value, in place', () => {
    const foreign = [
      '!approval::*',
      { function: 'shell::*', action: 'allow', modes: ['auto'] },
      'state::get',
      42,
      null,
      { nope: true },
      // biome-ignore lint/suspicious/noTemplateCurlyInString: the gate's ${VAR} text, kept as written
      { function: 'web::fetch', action: 'allow', args: { url: { matches: '${ALLOWED_HOST}' } } },
      rule(A('list_teams')),
    ]
    const rules = [...foreign.slice(0, 3), rule(L('list_issues')), rule(L('get_issue')), ...foreign.slice(3)]
    const before = JSON.stringify(rules)
    const out = m(rules, [L('whoami')])
    expect(out).toEqual([...foreign.slice(0, 3), rule(L('whoami')), ...foreign.slice(3)])
    for (const [i, item] of foreign.entries()) expect(out[i < 3 ? i : i + 1]).toBe(item)
    expect(JSON.stringify(rules)).toBe(before)
  })

  it('replaces a block where its first rule was, even a split one', () => {
    expect(m(['a::*', rule(L('get_issue')), 'b::*', rule(L('list_issues')), '!*'], [L('whoami')])).toEqual([
      'a::*',
      rule(L('whoami')),
      'b::*',
      '!*',
    ])
  })

  it('goes before the first unconditional catch-all, and only that', () => {
    for (const all of [
      '*',
      '!*',
      { function: '*', action: 'deny' },
      { function: '*', action: 'allow', rule_id: 'x' },
    ]) {
      expect(m(['a::*', all, 'b::*'], [L('whoami')])).toEqual(['a::*', rule(L('whoami')), all, 'b::*'])
    }
    for (const scoped of [
      { function: '*', action: 'allow', modes: ['auto'] },
      { function: '*', action: 'deny', args: { path: { equals: '/' } } },
      'mcp::*',
      '!mcp::linear::*',
    ]) {
      expect(m(['a::*', scoped], [L('whoami')])).toEqual(['a::*', scoped, rule(L('whoami'))])
    }
  })

  it('removes the block with an empty read set', () => {
    expect(m(['!approval::*', rule(L('whoami')), rule(A('whoami'))], [])).toEqual(['!approval::*', rule(A('whoami'))])
  })

  it('never mixes linear with linear-app', () => {
    const rules = [rule(A('list_teams')), rule(A('whoami'))]
    expect(blockOf(rules, 'linear')).toEqual({ ids: [], firstIndex: -1 })
    expect(blockOf(rules, 'linear-app')).toEqual({ ids: [A('list_teams'), A('whoami')], firstIndex: 0 })
    expect(m(rules, [L('whoami')])).toEqual([...rules, rule(L('whoami'))])
    expect(m(rules, [])).toEqual(rules)
  })

  it('writes exact ids of this server only', () => {
    expect(m([], [L('list_*'), A('whoami'), 'shell::exec', L('whoami'), L('whoami')])).toEqual([rule(L('whoami'))])
  })
})

describe('mergeGateEntry', () => {
  it('keeps every other key as read and adds none', () => {
    const value = { default_mode: 'manual', grant_reask_limit: 3, rules: ['!approval::*'] }
    expect(mergeGateEntry(value, 'linear', [L('whoami')])).toEqual({
      ok: true,
      value: { default_mode: 'manual', grant_reask_limit: 3, rules: ['!approval::*', rule(L('whoami'))] },
    })
    expect(value.rules).toEqual(['!approval::*'])
    const empty = mergeGateEntry({ rules: [] }, 'linear', [])
    expect(empty.ok).toBe(true)
    expect(Object.keys(empty.ok ? empty.value : {})).toEqual(['rules'])
  })

  it('refuses a shape it does not know', () => {
    for (const value of [null, undefined, 'x', 7, []]) {
      expect(mergeGateEntry(value, 'linear', [L('whoami')])).toEqual({
        ok: false,
        reason: 'The approval-gate entry is not an object.',
      })
    }
    for (const value of [{}, { rules: null }, { rules: 'x' }, { rules: {} }]) {
      expect(mergeGateEntry(value, 'linear', [L('whoami')])).toEqual({
        ok: false,
        reason: 'The approval-gate entry has no rules array.',
      })
    }
  })
})

describe('orphanBlocks', () => {
  it('counts the blocks of servers that are not configured', () => {
    const rules = [
      rule('mcp::old-name::a'),
      rule(L('whoami')),
      rule('mcp::old-name::b'),
      '!approval::*',
      { rule_id: 7 },
    ]
    expect(orphanBlocks(rules, ['linear'])).toEqual([{ server: 'old-name', count: 2 }])
    expect(orphanBlocks(rules, ['linear', 'old-name'])).toEqual([])
  })
})

describe('gateStatus', () => {
  it('no gate: warn, and never ok', () => {
    expect(st(null, {}, undefined, false)).toMatchObject({
      kind: 'no_gate',
      tone: 'warn',
      rowText: 'No approval gate · writes open',
      rowAction: false,
    })
    expect(st(present, {}, undefined, false).tone).not.toBe('ok')
  })

  it('unreadable', () => {
    expect(st(null)).toMatchObject({
      kind: 'unreadable',
      tone: 'alert',
      rowText: 'Cannot read the approval-gate rules',
      rowAction: false,
    })
  })

  it('not connected yet never judges the stored rules', () => {
    expect(
      gateStatus({
        gate: { installed: true, rules: [rule(L('gone')), rule(L('save_issue'))] },
        server: srv({ tools_listed: false, state: 'connecting' }),
        access: [],
      }),
    ).toMatchObject({
      kind: 'not_connected',
      all: ['not_connected'],
      tone: 'ghost',
      rowText: 'Connect to check',
      toAdd: [],
      toRemove: [],
      unsafe: [],
      allowed: 2,
      rowAction: false,
    })
  })

  it('no rules', () => {
    expect(st(['!approval::*'])).toMatchObject({
      kind: 'no_rules',
      all: ['no_rules'],
      tone: 'ghost',
      rowText: 'Every call asks',
      toAdd: [L('get_issue'), L('list_issues')],
      allowed: 0,
      ask: 4,
      destructive: 1,
    })
  })

  it('present', () => {
    expect(st(present)).toMatchObject({
      kind: 'present',
      all: ['present'],
      tone: 'ok',
      rowText: 'Reads allowed · writes ask',
      toAdd: [],
      toRemove: [],
      allowed: 2,
      ask: 2,
      destructive: 1,
      rowAction: false,
    })
  })

  it('missing', () => {
    expect(st([rule(L('get_issue'))])).toMatchObject({
      kind: 'missing',
      tone: 'warn',
      rowText: '1 new read tool still asks',
      toAdd: [L('list_issues')],
      allowed: 1,
      ask: 3,
    })
    const more = gateStatus({
      gate: { installed: true, rules: [rule(L('get_issue'))] },
      server: srv(),
      access: [...access, fa('whoami', 'read', 'name')],
    })
    expect(more.rowText).toBe('2 new read tools still ask')
  })

  it('out of date, harmless', () => {
    expect(st([...present, rule(L('gone'))])).toMatchObject({
      kind: 'stale_harmless',
      tone: 'warn',
      rowText: '1 rule for a removed tool',
      toRemove: [L('gone')],
      unsafe: [],
      rowAction: false,
    })
    expect(st([...present, rule(L('gone')), rule(L('gone2'))]).rowText).toBe('2 rules for removed tools')
  })

  it('out of date, unsafe', () => {
    expect(st([...present, rule(L('save_issue'))])).toMatchObject({
      kind: 'stale_unsafe',
      tone: 'alert',
      rowText: '1 write allowed without asking',
      unsafe: [L('save_issue')],
      rowAction: true,
    })
    expect(st([...present, rule(L('save_issue')), rule(L('delete_issue'))]).rowText).toBe(
      '2 writes allowed without asking',
    )
    expect(st([...present, rule(L('save_issue'))], { state: 'unavailable' }).rowAction).toBe(false)
  })

  it('a glob edited into the block is unsafe', () => {
    expect(
      st([...present, { function: 'mcp::linear::*', action: 'allow', rule_id: 'mcp-client:mcp::linear::list_x' }]),
    ).toMatchObject({ kind: 'stale_unsafe', unsafe: ['mcp::linear::*'] })
  })

  it('overridden, from the gate verdicts', () => {
    const held: Verdict = { verdict: 'needs_approval' }
    const deny: Verdict = {
      verdict: 'deny',
      reason: 'mcp::linear::get_issue is denied by permission rule mcp::linear::get_issue',
    }
    const base = {
      [L('list_issues')]: { verdict: 'allow' },
      [L('save_issue')]: held,
      [L('delete_issue')]: held,
    } as const
    expect(st(present, {}, { ...base, [L('get_issue')]: deny })).toMatchObject({
      kind: 'overridden_read',
      all: ['overridden_read', 'present'],
      tone: 'warn',
      rowText: 'Overridden in the approval gate',
      overridden: [{ functionId: L('get_issue'), verdict: deny }],
      rowAction: false,
    })
    expect(
      st(present, {}, { ...base, [L('get_issue')]: { verdict: 'allow' }, [L('save_issue')]: { verdict: 'allow' } }),
    ).toMatchObject({
      kind: 'overridden_write',
      all: ['overridden_write', 'present'],
      tone: 'alert',
      rowText: 'Overridden in the approval gate',
      rowAction: true,
    })
    // A read with no rule that is held is Missing, not Overridden.
    expect(
      st([rule(L('get_issue'))], {}, { [L('get_issue')]: { verdict: 'allow' }, [L('list_issues')]: held }).all,
    ).toEqual(['missing'])
    // A gate that holds nothing (default_mode full, or a catch-all allow) is a write allowed, with or without a block.
    expect(st([], {}, { [L('save_issue')]: { verdict: 'allow' } }).all).toEqual(['overridden_write', 'no_rules'])
  })

  it('shows the most severe status first', () => {
    const s = st(
      [rule(L('get_issue')), rule(L('save_issue'))],
      {},
      {
        [L('delete_issue')]: { verdict: 'allow' },
        [L('save_issue')]: { verdict: 'allow' },
      },
    )
    expect(s.all).toEqual(['stale_unsafe', 'overridden_write', 'missing'])
    expect(s.kind).toBe('stale_unsafe')
    // save_issue is already in unsafe.
    expect(s.overridden.map((o) => o.functionId)).toEqual([L('delete_issue')])
  })
})
