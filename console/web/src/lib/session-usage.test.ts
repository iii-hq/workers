import { describe, expect, it } from 'vitest'
import type { Usage } from '@/lib/sessions/types'
import type { Message } from '@/types/chat'
import {
  formatSpan,
  formatUsageValue,
  normalizeUsage,
  parseTurnId,
  reportedValue,
  sessionUsage,
  turnUsageByAnchor,
  turnUsages,
} from './session-usage'

const user = (id: string, at = 0, turnId?: string): Message => ({
  id,
  role: 'user',
  content: 'hi',
  createdAt: at,
  ...(turnId ? { turnId } : {}),
})

const assistant = (
  id: string,
  content: string,
  opts: {
    at?: number
    usage?: Usage
    turnId?: string
    streaming?: boolean
  } = {},
): Message => ({
  id,
  role: 'assistant',
  content,
  createdAt: opts.at ?? 0,
  ...(opts.usage ? { usage: opts.usage } : {}),
  ...(opts.turnId ? { turnId: opts.turnId } : {}),
  ...(opts.streaming ? { streaming: true } : {}),
})

const fcall = (
  id: string,
  opts: { at?: number; usage?: Usage; error?: boolean } = {},
): Message => ({
  id,
  role: 'function-trigger',
  functionId: 'shell::exec',
  input: {},
  output: opts.error ? { error: { kind: 'function_error' } } : { ok: true },
  createdAt: opts.at ?? 0,
  ...(opts.usage ? { usage: opts.usage } : {}),
})

describe('parseTurnId', () => {
  // Mirrors harness/src/ids.rs `assistant_entry_id(turn_id, step)`.
  it('recovers the turn id from a harness assistant entry id', () => {
    expect(parseTurnId('e_t_abc123_0_assistant')).toBe('t_abc123')
    expect(parseTurnId('e_t_abc123_12_assistant')).toBe('t_abc123')
  })

  it('splits on the LAST separator so underscore-bearing turn ids survive', () => {
    // `harness/src/ids.rs:118` asserts assistant_entry_id("t_1", 3) is
    // "e_t_1_3_assistant" — splitting on the first `_` would yield "t".
    expect(parseTurnId('e_t_1_3_assistant')).toBe('t_1')
    expect(parseTurnId('e_t_a_b_c_7_assistant')).toBe('t_a_b_c')
  })

  it('ignores a segment-index suffix added by the mapper', () => {
    expect(parseTurnId('e_t_abc123_0_assistant:2')).toBe('t_abc123')
  })

  it('returns undefined for ids that are not assistant entries', () => {
    expect(parseTurnId('e_t_abc_fc_call1')).toBeUndefined()
    expect(parseTurnId('e_notify_xyz')).toBeUndefined()
    expect(parseTurnId('local-optimistic-1')).toBeUndefined()
    expect(parseTurnId('e__assistant')).toBeUndefined()
  })
})

describe('normalizeUsage', () => {
  it('drops an object with no usable numbers', () => {
    expect(normalizeUsage(undefined)).toBeUndefined()
    expect(normalizeUsage({})).toBeUndefined()
    expect(normalizeUsage({ input: undefined })).toBeUndefined()
  })

  it('keeps an object with a zero value — zero is reported, not absent', () => {
    expect(normalizeUsage({ cache_read: 0 })).toEqual({ cache_read: 0 })
  })
})

describe('sessionUsage totals', () => {
  it('totals input + output only, excluding cache and reasoning', () => {
    // An OpenAI-shaped payload: `input` already INCLUDES cache_read and
    // `output` already includes reasoning. Adding them would double-count.
    const messages = [
      user('u1'),
      assistant('e_t_1_0_assistant', 'hi', {
        usage: {
          input: 1000,
          output: 200,
          cache_read: 800,
          reasoning: 150,
          cost_usd: 0.01,
        },
      }),
    ]
    const { totals } = sessionUsage(messages)
    expect(totals.input).toBe(1000)
    expect(totals.output).toBe(200)
    expect(totals.total).toBe(1200)
    expect(totals.cacheRead).toBe(800)
    expect(totals.reasoning).toBe(150)
  })

  it('applies the same total rule to an anthropic-shaped payload', () => {
    // Anthropic's `input` EXCLUDES cached tokens. `total` stays input+output
    // for both providers — the number means "billed in/out", not "prompt size".
    const messages = [
      user('u1'),
      assistant('e_t_1_0_assistant', 'hi', {
        usage: {
          input: 200,
          output: 200,
          cache_read: 800,
          cache_write: 100,
        },
      }),
    ]
    const { totals } = sessionUsage(messages)
    expect(totals.total).toBe(400)
    expect(totals.cacheWrite).toBe(100)
  })

  it('sums across steps but counts one entry once across its segments', () => {
    const usage: Usage = { input: 10, output: 2 }
    const messages = [
      user('u1'),
      // Two segments from the SAME entry — mapper attaches usage to the first.
      assistant('e_t_1_0_assistant:0', 'part one', { usage }),
      assistant('e_t_1_0_assistant:1', 'part two'),
      // A second step: a separate provider request, so it sums.
      assistant('e_t_1_1_assistant', 'done', {
        usage: { input: 30, output: 4 },
      }),
    ]
    const { totals } = sessionUsage(messages)
    expect(totals.input).toBe(40)
    expect(totals.output).toBe(6)
    expect(totals.total).toBe(46)
  })
})

describe('reported vs zero', () => {
  it('renders an unreported field as em dash, not zero', () => {
    // codex never reports cache_write; anthropic never reports reasoning.
    const messages = [
      user('u1'),
      assistant('e_t_1_0_assistant', 'hi', { usage: { input: 5, output: 1 } }),
    ]
    const { totals } = sessionUsage(messages)
    expect(totals.reported.cacheWrite).toBe(0)
    expect(reportedValue(totals, 'cacheWrite', totals.cacheWrite)).toBe('—')
    expect(reportedValue(totals, 'input', totals.input)).toBe('5')
  })

  it('renders a reported zero as 0', () => {
    const messages = [
      user('u1'),
      assistant('e_t_1_0_assistant', 'hi', {
        usage: { input: 5, output: 1, cache_read: 0 },
      }),
    ]
    const { totals } = sessionUsage(messages)
    expect(totals.reported.cacheRead).toBe(1)
    expect(reportedValue(totals, 'cacheRead', totals.cacheRead)).toBe('0')
  })
})

describe('steps and missing usage', () => {
  it('counts model calls and how many reported no usage', () => {
    const messages = [
      user('u1'),
      assistant('e_t_1_0_assistant', 'thinking', {
        usage: { input: 10, output: 2 },
      }),
      fcall('e_t_1_fc1'),
      assistant('e_t_1_1_assistant', 'done'),
    ]
    const usage = sessionUsage(messages)
    expect(usage.steps).toBe(3)
    expect(usage.stepsMissingUsage).toBe(2)
    expect(usage.functionCalls).toBe(1)
  })

  it('counts errored function triggers', () => {
    const messages = [
      user('u1'),
      fcall('e_t_1_fc1', { error: true }),
      fcall('e_t_1_fc2'),
    ]
    const usage = sessionUsage(messages)
    expect(usage.functionCalls).toBe(2)
    expect(usage.functionCallErrors).toBe(1)
  })

  it('exposes the last call raw and unsummed for the ctx cross-check', () => {
    const messages = [
      user('u1'),
      assistant('e_t_1_0_assistant', 'a', { usage: { input: 10, output: 2 } }),
      assistant('e_t_1_1_assistant', 'b', {
        at: 5,
        usage: { input: 90, output: 4 },
      }),
    ]
    const { lastCall } = sessionUsage(messages)
    expect(lastCall?.usage.input).toBe(90)
  })
})

describe('turn grouping', () => {
  it('counts a harness turn once, prompt included', () => {
    // The mapper stamps turnId only on assistant entries, so the user message
    // arrives without one. If it opened its own bucket every turn would be
    // counted twice.
    const messages = [
      { id: 'e_idem_x', role: 'user' as const, content: 'hi', createdAt: 0 },
      assistant('e_t_9_0_assistant', 'done', {
        at: 1,
        turnId: 't_9',
        usage: { input: 10, output: 2 },
      }),
    ]
    const turns = turnUsages(messages)
    expect(turns).toHaveLength(1)
    expect(turns[0].turnId).toBe('t_9')
    // The prompt's timestamp anchors the turn's start.
    expect(turns[0].startedAt).toBe(0)
  })

  it('keeps consecutive harness turns separate', () => {
    const messages = [
      user('e_idem_1', 0),
      assistant('e_t_1_0_assistant', 'a', {
        at: 1,
        turnId: 't_1',
        usage: { input: 10, output: 2 },
      }),
      user('e_idem_2', 2),
      assistant('e_t_2_0_assistant', 'b', {
        at: 3,
        turnId: 't_2',
        usage: { input: 20, output: 4 },
      }),
    ]
    const turns = turnUsages(messages)
    expect(turns.map((t) => t.turnId)).toEqual(['t_1', 't_2'])
  })

  it('opens a synthetic turn for a prompt still awaiting its reply', () => {
    const messages = [
      user('e_idem_1', 0),
      assistant('e_t_1_0_assistant', 'a', { at: 1, turnId: 't_1' }),
      // Optimistic send: no assistant entry exists yet.
      user('local-pending', 2),
    ]
    const turns = turnUsages(messages)
    expect(turns).toHaveLength(2)
    expect(turns[1].turnId).toMatch(/^local:/)
  })

  it('groups steps of one tool loop into a single turn', () => {
    const messages = [
      user('u1', 0),
      assistant('e_t_9_0_assistant', '', {
        at: 1,
        usage: { input: 100, output: 10 },
      }),
      fcall('e_t_9_fc1', { at: 2 }),
      assistant('e_t_9_1_assistant', 'all done', {
        at: 3,
        usage: { input: 200, output: 20 },
      }),
    ]
    const turns = turnUsages(messages)
    const withUsage = turns.filter((t) => t.steps > 0)
    expect(withUsage).toHaveLength(1)
    const turn = withUsage[0]
    expect(turn.turnId).toBe('t_9')
    expect(turn.steps).toBe(2)
    expect(turn.totals.total).toBe(330)
    expect(turn.functionCalls).toBe(1)
    // Spans the prompt (t=0) through the closing reply (t=3).
    expect(turn.durationMs).toBe(3)
  })

  it('prefers a mapper-supplied turnId over the parsed entry id', () => {
    const messages = [
      user('u1'),
      assistant('e_t_parsed_0_assistant', 'hi', {
        turnId: 't_from_origin',
        usage: { input: 1, output: 1 },
      }),
    ]
    expect(turnUsages(messages).at(-1)?.turnId).toBe('t_from_origin')
  })

  it('falls back to a user-message boundary when no turn id exists', () => {
    const messages = [
      user('u1', 0),
      assistant('local-1', 'first'),
      user('u2', 10),
      assistant('local-2', 'second'),
    ]
    expect(turnUsages(messages)).toHaveLength(2)
  })

  it('carries tokens from a tool-only turn but gives it no anchor', () => {
    // An assistant entry with only function calls emits no prose segment,
    // yet it cost a real request. Its tokens must still count.
    const messages = [
      user('u1'),
      fcall('e_t_5_fc1', { usage: { input: 500, output: 25 } }),
    ]
    const turn = turnUsages(messages).at(-1)
    expect(turn?.totals.total).toBe(525)
    expect(turn?.anchorId).toBeUndefined()
    expect(turnUsageByAnchor(messages).size).toBe(0)
  })

  it('anchors the chip on the turn last prose segment', () => {
    const messages = [
      user('u1'),
      assistant('e_t_7_0_assistant', 'first', {
        usage: { input: 1, output: 1 },
      }),
      assistant('e_t_7_1_assistant', 'last', {
        usage: { input: 2, output: 2 },
      }),
    ]
    const byAnchor = turnUsageByAnchor(messages)
    expect(byAnchor.get('e_t_7_1_assistant')?.totals.total).toBe(6)
    expect(byAnchor.has('e_t_7_0_assistant')).toBe(false)
  })

  it('marks a turn streaming while any of its segments is', () => {
    const messages = [
      user('u1'),
      assistant('e_t_3_0_assistant', 'partial', { streaming: true }),
    ]
    expect(turnUsages(messages).at(-1)?.streaming).toBe(true)
  })
})

describe('empty and degenerate input', () => {
  it('handles an empty transcript', () => {
    const usage = sessionUsage([])
    expect(usage.steps).toBe(0)
    expect(usage.totals.total).toBe(0)
    expect(usage.turns).toEqual([])
    expect(usage.durationMs).toBe(0)
    expect(usage.lastCall).toBeUndefined()
  })

  it('handles a transcript with no usage at all (pre-fix sessions)', () => {
    const messages = [user('u1'), assistant('e_t_1_0_assistant', 'hi')]
    const usage = sessionUsage(messages)
    expect(usage.totals.total).toBe(0)
    expect(reportedValue(usage.totals, 'input', usage.totals.input)).toBe('—')
    expect(usage.steps).toBe(1)
    expect(usage.stepsMissingUsage).toBe(1)
  })
})

describe('formatting', () => {
  it('formats durations like eval formatMetric', () => {
    expect(formatUsageValue(450, 'duration')).toBe('450ms')
    expect(formatUsageValue(1500, 'duration')).toBe('1.50s')
  })

  it('formats cost to six decimals like eval', () => {
    expect(formatUsageValue(0.0482, 'cost')).toBe('$0.048200')
  })

  it('formats token counts with the console dialect', () => {
    expect(formatUsageValue(412908, 'tokens')).toBe('413k')
    expect(formatUsageValue(900, 'tokens')).toBe('900')
  })

  it('returns em dash for undefined and non-finite', () => {
    expect(formatUsageValue(undefined)).toBe('—')
    expect(formatUsageValue(Number.NaN)).toBe('—')
  })

  it('formats session spans', () => {
    expect(formatSpan(38_000)).toBe('38s')
    expect(formatSpan(724_000)).toBe('12m 04s')
    expect(formatSpan(14_820_000)).toBe('4h 07m')
    expect(formatSpan(0)).toBe('—')
  })
})
