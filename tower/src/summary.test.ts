import { describe, expect, it } from 'vitest'
import { towerFixture, towerSummaryFixture } from './fixtures.js'
import {
  acceptSummary,
  resolveSummary,
  SUMMARY_MAX_AGE,
  summaryIsFresh,
} from './summary.js'

const result = towerSummaryFixture
const checked = Date.parse(result.checked_at)

describe('tower writer result contract', () => {
  it('accepts the kit contract, including names with digits and the empty scope', () => {
    expect(acceptSummary(result, null, checked)).toEqual(result)
    expect(
      acceptSummary(
        {
          ...result,
          brief: { headline: '', text: '', refs: [] },
          diary: [],
          now: {},
          pr_waits: {},
        },
        null,
        checked,
      ),
    ).not.toBeNull()
  })

  it.each([
    null,
    JSON.stringify(result),
    { ...result, extra: true },
    { ...result, unchanged: 'false' },
    { ...result, checked_at: '2026-02-30T12:00:00Z' },
    { ...result, checked_at: '2026-11-22T12:00Z' },
    { ...result, checked_at: '2026-11-22T12:00:00+00:00' },
    { ...result, brief: { ...result.brief, refs: ['https://example.com'] } },
    { ...result, brief: { ...result.brief, refs: ['KAN-616', 'KAN-616'] } },
    { ...result, brief: { ...result.brief, refs: [] } },
    { ...result, diary: Array(51).fill(result.diary[0]) },
    { ...result, diary: [{ ...result.diary[0], at: 'invalid' }] },
    {
      ...result,
      now: { 'session:gone': { text: 'Waits.', refs: ['KAN-616'] } },
    },
    {
      ...result,
      now: { 'bad subject': { text: 'Waits.', refs: ['KAN-616'] } },
    },
    {
      ...result,
      pr_waits: { 'owner/repo#0': { text: 'Waits.', refs: ['KAN-616'] } },
    },
    {
      ...result,
      pr_waits: { 'owner/repo#1': { text: '', refs: ['owner/repo#1'] } },
    },
    {
      ...result,
      now: {
        'session:gone': { text: 'Waits.', refs: ['session:gone'], extra: true },
      },
    },
  ])(
    'keeps the previous prose and checkpoint on an invalid result (%#)',
    (value) => {
      expect(acceptSummary(value, result, checked)).toBe(result)
    },
  )

  it('rejects future checkpoints, future milestones and out-of-order results', () => {
    expect(acceptSummary(result, null, checked - 1)).toBeNull()
    expect(
      acceptSummary(
        {
          ...result,
          diary: [
            { ...result.diary[0], at: new Date(checked + 1).toISOString() },
          ],
        },
        result,
        checked,
      ),
    ).toBe(result)
    expect(
      acceptSummary(
        { ...result, checked_at: new Date(checked - 1).toISOString() },
        result,
        checked,
      ),
    ).toBe(result)
  })

  it('keeps the previous summary if milestones are duplicated or are not newest first', () => {
    const entry = result.diary[0]
    const earlier = {
      ...entry,
      at: new Date(Date.parse(entry.at) - 60_000).toISOString(),
    }
    expect(
      acceptSummary({ ...result, diary: [entry, entry] }, result, checked),
    ).toBe(result)
    expect(
      acceptSummary({ ...result, diary: [earlier, entry] }, result, checked),
    ).toBe(result)
    expect(
      acceptSummary({ ...result, diary: [entry, earlier] }, result, checked),
    ).not.toBe(result)
  })

  it('advances an unchanged checkpoint while keeping accepted prose and restores a carried result on reload', () => {
    const next = {
      ...result,
      checked_at: new Date(checked + 60_000).toISOString(),
      unchanged: true,
    }
    const previous = {
      ...result,
      brief: { ...result.brief, text: 'Previously accepted text.' },
    }
    expect(acceptSummary(next, previous, checked + 60_000)).toEqual({
      ...previous,
      checked_at: next.checked_at,
      unchanged: true,
    })
    expect(acceptSummary(next, null, checked + 60_000)).toEqual(next)
  })
})

describe('live references and freshness', () => {
  it('drops unresolved refs and notes about sessions and PRs no longer live without changing the source result', () => {
    const input = {
      ...result,
      brief: {
        ...result.brief,
        refs: [
          'KAN-616',
          'session:dispatch-r1',
          'iii-hq/iii-cloud#451',
          'KAN-99999',
          'session:gone',
          'owner/repo#999',
        ],
      },
      diary: [{ ...result.diary[0], refs: ['KAN-635', 'KAN-99999'] }],
      now: {
        ...result.now,
        'session:gone': { text: 'Gone.', refs: ['session:gone'] },
      },
      pr_waits: {
        ...result.pr_waits,
        'owner/repo#999': { text: 'Gone.', refs: ['owner/repo#999'] },
      },
    }
    const resolved = resolveSummary(input, towerFixture)
    expect(resolved.brief.refs).toEqual([
      'KAN-616',
      'session:dispatch-r1',
      'iii-hq/iii-cloud#451',
    ])
    expect(resolved.diary[0].refs).toEqual(['KAN-635'])
    expect(Object.keys(resolved.now)).toEqual(['session:dispatch-r1'])
    expect(Object.keys(resolved.pr_waits)).toEqual(['iii-hq/iii-cloud#451'])
    expect(input.brief.refs).toContain('KAN-99999')
    const gone = resolveSummary(input, { tickets: [], runs: [], prs: [] })
    expect(gone.brief.refs).toEqual([])
    expect(gone.now).toEqual({})
    expect(gone.pr_waits).toEqual({})
  })

  it('is fresh through thirty minutes and stale afterward, including invalid/future timestamps', () => {
    expect(summaryIsFresh(checked, checked)).toBe(true)
    expect(summaryIsFresh(checked, checked + SUMMARY_MAX_AGE)).toBe(true)
    expect(summaryIsFresh(checked, checked + SUMMARY_MAX_AGE + 1)).toBe(false)
    expect(summaryIsFresh(checked, checked - 1)).toBe(false)
    expect(summaryIsFresh(Number.NaN, checked)).toBe(false)
  })
})
