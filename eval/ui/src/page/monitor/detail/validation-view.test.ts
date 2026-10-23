import { describe, expect, it } from 'vitest'
import type { E2eExecution, E2eScenario, ValidationLink } from '../../../types'
import {
  checkLabel,
  checkValue,
  difference,
  formatClock,
  foundSummary,
  harnessNote,
  hasMeasures,
  latestLink,
  mismatches,
  noRunsNotice,
  planHint,
  reportProblem,
  scenarioTables,
  signalRow,
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

describe('measures per scenario', () => {
  const cohort = (over: Partial<E2eScenario> = {}) =>
    scenario({ pass_rate: 1, p50_function_calls: 11, median_wall_time_ms: 15_100, ...over })
  const rowsOf = (table: ReturnType<typeof scenarioTables>[number]) =>
    Object.fromEntries(table.rows.map((r) => [r.key, r]))

  it('puts the target scenario first and never averages scenarios together', () => {
    const both = (id: string) => run({ scenarios: [scenario({ scenario_id: 'other' }), scenario({ scenario_id: id })] })
    const tables = scenarioTables(link(both('tool_contract_recovery'), both('tool_contract_recovery')), {
      target: 'tool_contract_recovery',
    })
    expect(tables.map((table) => [table.scenarioId, table.target])).toEqual([
      ['tool_contract_recovery', true],
      ['other', false],
    ])
    expect(tables[0].runs).toBe('5 runs per side')
  })

  it('computes the difference in code: absolute, relative, and points for a rate', () => {
    const baseline = run({ scenarios: [cohort()] })
    const candidate = run({
      execution_id: 'exe_2',
      scenarios: [
        cohort({
          pass_rate: 0.6,
          p50_function_calls: 8,
          median_wall_time_ms: 14_600,
          measures: { ...scenario().measures, cost_usd: { average: 0.17, samples: 5 } },
        }),
      ],
    })
    const [table] = scenarioTables(link(baseline, candidate), {
      target: 'tool_contract_recovery',
      primary: 'function_calls',
    })
    const rows = rowsOf(table)
    expect(rows.pass_rate.delta).toEqual({ abs: '-40 pp' })
    expect(rows.function_calls.qualifier).toBe('median per run')
    expect(rows.function_calls.delta).toEqual({ abs: '-3', pct: '-27 %' })
    expect(rows.function_calls.primary).toBe(true)
    expect(rows.cost_usd.primary).toBeUndefined()
    expect(rows.cost_usd.delta?.abs).toBe('-$0.0400')
    expect(rows.duration.delta?.abs).toBe('-500ms')
  })

  it('leaves the difference blank when a side reports nothing, and says so', () => {
    const noCost = cohort({ measures: { ...scenario().measures, cost_usd: { samples: 0 } } })
    const [table] = scenarioTables(link(run({ scenarios: [cohort()] }), run({ scenarios: [noCost] })), {})
    const cost = rowsOf(table).cost_usd
    expect(cost.delta).toBeUndefined()
    expect(cost.candidate).toEqual({ warn: 'Not reported' })
    expect(cost.baseline).toEqual({ value: '$0.2100', sub: '3 of 5 runs', warn: 'Not reported in 2' })
  })

  it('shows the cohort cost without a sample count the measure never had (new plan executions)', () => {
    // The E2E reports tokens only in `scenario_metrics`: no cost measure, a cost per run from the cohort.
    const planned = (cost: number) =>
      cohort({ measures: { ...scenario().measures, cost_usd: { samples: 0 } }, cost_usd_per_run: cost })
    const [table] = scenarioTables(
      link(run({ scenarios: [planned(0.0039)] }), run({ scenarios: [planned(0.0037)] })),
      {},
    )
    const cost = rowsOf(table).cost_usd
    expect(cost.baseline).toEqual({ value: '$0.0039', sub: undefined, warn: undefined })
    expect(cost.candidate).toEqual({ value: '$0.0037', sub: undefined, warn: undefined })
    expect(cost.delta?.abs).toBe('-$0.0002')
  })

  it('says a handful of runs is descriptive only, and counts one run in the singular', () => {
    const few = (runs: number) => run({ scenarios: [scenario({ run_count: runs })] })
    const [one] = scenarioTables(link(few(1), few(1)), {})
    expect(one.runs).toBe('1 run per side')
    expect(one.caution).toBe('n=1 per side; the E2E advises at least 5, so these differences are descriptive only')
    const [uneven] = scenarioTables(link(few(5), few(2)), {})
    expect(uneven.caution).toMatch(/^n=5 and 2; the E2E advises at least 5/)
    expect(scenarioTables(link(few(5), few(5)), {})[0].caution).toBeUndefined()
    expect(scenarioTables(link(few(20), few(7)), {})[0].caution).toBeUndefined()
  })

  it('compares only like with like: a median against a mean has no difference', () => {
    const [table] = scenarioTables(
      link(run({ scenarios: [cohort()] }), run({ scenarios: [cohort({ p50_function_calls: undefined })] })),
      {},
    )
    const calls = rowsOf(table).function_calls
    expect(calls.qualifier).toBe('mean per run')
    expect(calls.delta).toEqual({ abs: '0', pct: '0 %' })
    const onlyMedian = scenarioTables(
      link(
        run({ scenarios: [cohort({ measures: { ...scenario().measures, function_calls: { samples: 0 } } })] }),
        run({ scenarios: [cohort({ p50_function_calls: undefined })] }),
      ),
      {},
    )[0]
    expect(rowsOf(onlyMedian).function_calls.delta).toBeUndefined()
    expect(rowsOf(onlyMedian).function_calls.baseline.sub).toBe('median per run')
  })

  it('has no relative change from a baseline of zero', () => {
    expect(difference(0, 3, String)).toEqual({ abs: '+3', pct: undefined })
    expect(difference(3, 0, String)).toEqual({ abs: '-3', pct: '-100 %' })
  })

  it('shows the signal the criterion counts, from the computed evidence, first in the target table', () => {
    const side = (mean: number) => ({ execution_id: 'e', runs: [], n: 5, mean })
    const evidence = {
      scenario_id: 'tool_contract_recovery',
      computed_at: 1,
      baseline: side(3),
      candidate: side(0),
      computed_outcome: 'validated_improvement' as const,
      reason: '',
    }
    const criterion = {
      metric: 'signal_per_run' as const,
      pattern: 'repeated_contract_discovery:engine::functions::info',
      direction: 'decrease' as const,
      min_effect: 0.5,
      min_runs: 5,
      scenario_id: 'tool_contract_recovery',
      registered_at: 1,
      registered_by: 'layon',
    }
    const row = signalRow(criterion, evidence)
    expect(row?.label).toBe('repeated_contract_discovery')
    expect(row?.delta).toEqual({ abs: '-3', pct: '-100 %' })
    expect(row?.baseline).toEqual({ value: '3', sub: 'n=5' })
    const [table] = scenarioTables(link(), { target: 'tool_contract_recovery', signal: row })
    expect(table.rows[0].key).toBe('signal')
    expect(signalRow({ ...criterion, metric: 'pass_rate' }, evidence)).toBeUndefined()
  })

  it('has nothing to show for an execution without scenarios', () => {
    const bare = run({ scenarios: [] })
    expect(scenarioTables(link(bare, bare), {})).toEqual([])
    expect(hasMeasures(link(bare, bare))).toBe(false)
    expect(hasMeasures(link())).toBe(true)
  })
})

describe('copy', () => {
  it('builds the plan hint from the scenario', () => {
    expect(planHint('tool_contract_recovery')).toMatch(/^Run `tool_contract_recovery` twice with the same model/)
    expect(planHint(null)).toMatch(/^This plan needs a new E2E case/)
    expect(noRunsNotice('s1').detail).toContain('Run s1 twice there')
    expect(noRunsNotice(null).detail).toContain('Run the case twice there')
  })
})
