import { describe, expect, it } from 'vitest'
import type { E2eExecution, E2eScenario, ValidationLink } from '../../../types'
import {
  aggregate,
  checkLabel,
  checkValue,
  formatClock,
  foundSummary,
  harnessNote,
  hasMeasures,
  latestLink,
  measureRows,
  mismatches,
  reportProblem,
} from './validation-view'

const scenario = (over: Partial<E2eScenario> = {}): E2eScenario => ({
  scenario_id: 'tool_contract_recovery',
  run_count: 5,
  measures: {
    function_calls: { average: 23, samples: 5 },
    function_call_errors: { average: 0, samples: 5 },
    tokens: { average: 61_300, samples: 5 },
    duration_seconds: { average: 134, samples: 5 },
    cost_usd: { average: 0.21, samples: 3 },
  },
  ...over,
})

const run = (over: Partial<E2eExecution> = {}): E2eExecution => ({
  execution_id: 'exe_1',
  reports_available: true,
  reports: [],
  scenarios: [scenario()],
  harness_version: '1.9.2',
  conclusion: 'passed',
  assessments: { passed: 8, total: 8 },
  ...over,
})

const link = (baseline = run(), candidate = run({ execution_id: 'exe_2' }), attached_at = 1): ValidationLink => ({
  suggestion_index: 0,
  baseline,
  candidate,
  comparability: { comparable: true, checks: [] },
  attached_at,
})

describe('identity checks', () => {
  it('labels the backend fields and keeps machine fields mono', () => {
    expect(checkLabel('scenarios')).toEqual({ text: 'Scenario', mono: false })
    expect(checkLabel('e2e_revision')).toEqual({ text: 'E2E revision', mono: false })
    expect(checkLabel('behavior_sha256')).toEqual({ text: 'behavior_sha256', mono: true })
    expect(checkLabel('something_new')).toEqual({ text: 'something_new', mono: true })
  })

  it('unpacks per-scenario values the way the backend joins them', () => {
    expect(checkValue('scenarios', 'a=, b=')).toBe('a, b')
    expect(checkValue('behavior_sha256', 'a=sha256:4be1c3d9aa07')).toBe('4be1…a07')
    expect(checkValue('contract_fingerprint', 'a=91af00000003d, b=77aa00000001c')).toBe('a=91af…03d, b=77aa…01c')
  })

  it('never invents a value for an unknown one', () => {
    expect(checkValue('model', undefined)).toBe('not reported')
    expect(checkValue('behavior_sha256', 'a=')).toBe('not reported')
    expect(checkValue('model', 'deepseek-v4-pro')).toBe('deepseek-v4-pro')
    expect(checkValue('e2e_revision', '0123456789abcdef0123456789abcdef01234567')).toBe('0123456')
    expect(checkValue('e2e_revision', 'r14')).toBe('r14')
  })

  it('lists only the checks that differ', () => {
    const checks = [
      { field: 'model', matches: true },
      { field: 'provider', matches: false, baseline: 'a', candidate: 'b' },
    ]
    expect(mismatches(checks).map((check) => check.field)).toEqual(['provider'])
  })
})

describe('runs', () => {
  it('summarises a found run', () => {
    expect(foundSummary(run())).toBe('tool_contract_recovery · harness 1.9.2')
    expect(foundSummary(run({ scenarios: [], harness_version: undefined, reports_available: false }))).toBe(
      'report unavailable',
    )
  })

  it('says whether the Harness version differs, as intended', () => {
    expect(harnessNote(run(), run({ harness_version: '1.9.3-rc.1' }))).toEqual({
      kind: 'differs',
      baseline: '1.9.2',
      candidate: '1.9.3-rc.1',
    })
    expect(harnessNote(run(), run())).toEqual({ kind: 'same', version: '1.9.2' })
    expect(harnessNote(run(), run({ harness_version: undefined }))).toEqual({ kind: 'unknown' })
  })

  it('picks the newest link and counts the earlier ones', () => {
    expect(latestLink([])).toBeNull()
    const found = latestLink([
      link(undefined, undefined, 5),
      link(undefined, undefined, 9),
      link(undefined, undefined, 2),
    ])
    expect(found?.latest.attached_at).toBe(9)
    expect(found?.earlier).toBe(2)
  })

  it('formats the attach time as HH:MM', () => {
    expect(formatClock(new Date(2026, 9, 2, 21, 5).getTime())).toBe('21:05')
  })

  it('reports the runs whose report is missing', () => {
    expect(reportProblem(link())).toBeNull()
    expect(reportProblem(link(run(), run({ execution_id: 'exe_2', reports_available: false })))?.ids).toEqual(['exe_2'])
    expect(reportProblem(link(run({ evidence_error: 'assets gone' })))).toEqual({
      ids: ['exe_1'],
      errors: ['assets gone'],
    })
  })
})

describe('measures', () => {
  it('weights the average of several scenarios by the runs that reported it', () => {
    const two = run({
      scenarios: [
        scenario({ measures: { ...scenario().measures, tokens: { average: 100, samples: 1 } } }),
        scenario({ scenario_id: 'other', measures: { ...scenario().measures, tokens: { average: 400, samples: 3 } } }),
      ],
    })
    expect(aggregate(two, 'tokens')).toEqual({ average: 325, samples: 4 })
  })

  it('treats no sample as no value, never zero', () => {
    const none = run({ scenarios: [scenario({ measures: { ...scenario().measures, cost_usd: { samples: 0 } } })] })
    expect(aggregate(none, 'cost_usd')).toEqual({ samples: 0 })
  })

  it('shows each execution on its own with its sample count and gaps', () => {
    const noCost = run({
      execution_id: 'exe_2',
      scenarios: [scenario({ measures: { ...scenario().measures, cost_usd: { samples: 0 } } })],
    })
    const rows = Object.fromEntries(measureRows(run(), noCost).map((row) => [row.key, row]))
    expect(rows.runs.baseline).toEqual({ value: '5' })
    expect(rows.assessments.baseline).toEqual({ value: '8 / 8', sub: 'conclusion: passed' })
    expect(rows.function_calls.baseline).toEqual({ value: '23', sub: '5 of 5 runs', warn: undefined })
    expect(rows.tokens.baseline.value).toBe('61.3k')
    expect(rows.duration_seconds.baseline.value).toBe('2m 14s')
    expect(rows.cost_usd.baseline).toEqual({
      value: '$0.21',
      sub: '3 of 5 runs',
      warn: 'Not reported in 2 of 5 runs',
    })
    expect(rows.cost_usd.candidate).toEqual({ warn: 'Not reported in 5 of 5 runs' })
  })

  it('leaves every cell empty for an execution without scenarios or assessments', () => {
    const bare = run({ scenarios: [], assessments: undefined, conclusion: undefined })
    expect(
      measureRows(bare, bare).every((row) => row.baseline.value === undefined && row.baseline.warn === undefined),
    ).toBe(true)
    expect(hasMeasures(link(bare, bare))).toBe(false)
    expect(hasMeasures(link())).toBe(true)
  })
})
