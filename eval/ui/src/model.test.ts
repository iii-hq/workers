import { describe, expect, it } from 'vitest'
import {
  briefMarkdown,
  entryLabel,
  formatCost,
  formatStamp,
  formatTokens,
  matchesFilter,
  monitorSummary,
  pipeline,
  planMarkdown,
  rowDetail,
  statusPresentation,
  suggestionsUsing,
} from './model'
import type { AnalysisRecord, Diagnostic, Snapshot, Suggestion } from './types'

const T0 = Date.UTC(2026, 9, 2, 19, 24, 0)

function record(overrides: Partial<AnalysisRecord> = {}): AnalysisRecord {
  return {
    schema_version: 1,
    evaluation_id: 'eval_1',
    observation_key: 'k',
    origin: 'automatic',
    session_id: 's_root',
    turn_id: 't_aaa',
    model: { model: 'm', provider: 'p' },
    config_revision: 'r',
    rules_version: 'session-monitor-rules/1',
    criteria_version: 'c',
    status: 'queued',
    step: 0,
    created_at: T0,
    updated_at: T0,
    deadline: T0 + 180_000,
    observe_since: T0,
    counters: { sessions: 0, entries: 0, diagnostics: 0, suggestions: 0, rejected_suggestions: 0, validations: 0 },
    stages: [{ status: 'queued', at: T0 }],
    usage: { judge_calls: 0, judge_input_tokens: 0, judge_output_tokens: 0, judge_usage_complete: false },
    ...overrides,
  }
}

const completed = record({
  status: 'completed',
  completed_at: T0 + 47_300,
  counters: { sessions: 3, entries: 252, diagnostics: 2, suggestions: 1, rejected_suggestions: 0, validations: 0 },
  routing: { investigate: true, reasons: ['diagnostics'] },
  stages: [
    { status: 'queued', at: T0 },
    { status: 'collecting', at: T0 + 200 },
    { status: 'judging', at: T0 + 2_300 },
    { status: 'investigating', at: T0 + 6_100 },
    { status: 'completed', at: T0 + 47_300 },
  ],
})

describe('status presentation', () => {
  it('describes the monitor, never the task', () => {
    expect(statusPresentation(completed)).toEqual({ label: '1 suggestion', tone: 'ok' })
    expect(statusPresentation(record({ status: 'completed' }))).toEqual({ label: 'No suggestions', tone: 'ok' })
    expect(
      statusPresentation(record({ status: 'collecting', pending_reason: 'descendant sessions are still running' })),
    ).toEqual({ label: 'Collecting · waiting', tone: 'warn' })
    expect(statusPresentation(record({ status: 'judging' }))).toEqual({ label: 'Triage', tone: 'accent' })
  })

  it('never shows a failure as healthy', () => {
    const failed = (code: string, stage: AnalysisRecord['status']) =>
      statusPresentation(record({ status: 'failed', failure: { code, stage, message: 'x' } }))
    expect(failed('judge_provider_unavailable', 'judging')).toEqual({ label: 'Failed at triage', tone: 'alert' })
    expect(failed('external_outcome_unknown', 'judging').label).toBe('Failed · outcome unknown')
    expect(failed('source_advanced', 'collecting').label).toBe('Failed · session changed')
    expect(failed('coverage_insufficient', 'collecting')).toEqual({ label: 'Insufficient evidence', tone: 'warn' })
  })
})

describe('row detail', () => {
  it('names the most useful fact', () => {
    expect(rowDetail(completed)).toBe('2 signals')
    expect(rowDetail(record({ status: 'collecting', pending_reason: 'descendant sessions are still running' }))).toBe(
      'children running',
    )
    // What routed an analysis is the detail's to say; a row names the signals.
    expect(rowDetail(record({ status: 'completed', routing: { investigate: true, reasons: ['audit_sample'] } }))).toBe(
      'no signals',
    )
    expect(
      rowDetail(
        record({ status: 'failed', failure: { code: 'judge_provider_unavailable', stage: 'judging', message: '' } }),
      ),
    ).toBe('provider unavailable')
    expect(
      rowDetail(
        record({
          status: 'cancelled',
          stages: [
            { status: 'queued', at: T0 },
            { status: 'investigating', at: T0 + 5 },
            { status: 'cancelled', at: T0 + 9 },
          ],
        }),
      ),
    ).toBe('during investigation')
  })
})

describe('pipeline', () => {
  it('times every finished step from the stage starts', () => {
    const steps = pipeline(completed, T0 + 60_000)
    expect(steps.map((step) => step.state)).toEqual(['done', 'done', 'done', 'done', 'done'])
    expect(steps.map((step) => step.durationMs)).toEqual([200, 2_100, 3_800, 41_200, 47_300])
  })

  it('marks the running, waiting, failed and skipped steps', () => {
    const waiting = pipeline(
      record({
        status: 'collecting',
        pending_reason: 'descendant sessions are still running',
        stages: [
          { status: 'queued', at: T0 },
          { status: 'collecting', at: T0 + 100 },
        ],
      }),
      T0 + 1_100,
    )
    expect(waiting.map((step) => step.state)).toEqual(['done', 'waiting', 'pending', 'pending', 'pending'])
    expect(waiting[1].durationMs).toBe(1_000)

    const failed = pipeline(
      record({
        status: 'failed',
        failure: { code: 'judge_provider_unavailable', stage: 'judging', message: '' },
        completed_at: T0 + 40_000,
        stages: [
          { status: 'queued', at: T0 },
          { status: 'collecting', at: T0 + 100 },
          { status: 'judging', at: T0 + 2_000 },
          { status: 'failed', at: T0 + 40_000 },
        ],
      }),
      T0 + 50_000,
    )
    expect(failed.map((step) => step.state)).toEqual(['done', 'done', 'failed', 'pending', 'failed'])

    const quiet = pipeline(
      record({
        status: 'completed',
        completed_at: T0 + 5_000,
        routing: { investigate: false, reasons: [] },
        stages: [
          { status: 'queued', at: T0 },
          { status: 'collecting', at: T0 + 100 },
          { status: 'judging', at: T0 + 2_000 },
          { status: 'completed', at: T0 + 5_000 },
        ],
      }),
      T0 + 6_000,
    )
    expect(quiet.map((step) => step.state)).toEqual(['done', 'done', 'done', 'skipped', 'done'])
    expect(quiet[2].durationMs).toBe(3_000)
  })
})

describe('filters and summary', () => {
  it('filters by what the user is looking for', () => {
    expect(matchesFilter(completed, 'suggestions')).toBe(true)
    expect(matchesFilter(completed, 'active')).toBe(false)
    expect(matchesFilter(record({ status: 'judging' }), 'active')).toBe(true)
  })

  it('counts running and pending work and keeps unknown cost unknown', () => {
    const now = T0 + 1_000
    const summary = monitorSummary(
      [
        record({ status: 'judging' }),
        record({ status: 'queued' }),
        { ...completed, usage: { ...completed.usage, llm_input_tokens: 9_000, llm_cost_usd: 0.084 } },
        { ...completed, usage: { ...completed.usage, llm_input_tokens: 9_000 } },
      ],
      now,
    )
    expect(summary).toMatchObject({ running: 1, pending: 1, today: 4, todayCostUnknown: 1 })
    expect(summary.todayCostUsd).toBeCloseTo(0.084)
  })
})

describe('formatting', () => {
  it('never turns unknown cost into zero', () => {
    expect(formatCost(undefined)).toBe('not reported')
    expect(formatCost(0.0851)).toBe('$0.0851')
    expect(formatTokens(61_300)).toBe('61.3k')
    expect(formatTokens(1_204)).toBe('1,204')
    expect(formatTokens(undefined)).toBe('—')
  })

  it('labels evidence chips from the preview the models saw', () => {
    const snapshot = {
      sessions: [
        {
          session_id: 's_root',
          preview: [
            { entry_id: 'e_t_aaa_c2', message: { role: 'function_result', function_id: 'engine::functions::info' } },
            { entry_id: 'e_t_aaa_1_notice_0', custom: { custom_type: 'model_notice' } },
          ],
        },
      ],
    } as unknown as Snapshot
    expect(entryLabel(snapshot, { session_id: 's_root', entry_id: 'e_t_aaa_c2' })).toBe('c2 · functions::info')
    expect(entryLabel(snapshot, { session_id: 's_root', entry_id: 'e_t_aaa_1_notice_0' })).toBe(
      '1_notice_0 · model_notice',
    )
    expect(entryLabel(snapshot, { session_id: 's_root', entry_id: 'e_t_aaa_7_assistant' })).toBe('7_assistant')
  })

  it('links signals to the suggestions that cite them', () => {
    const diagnostic = {
      evidence: [{ session_id: 's', entry_id: 'a' }],
    } as Diagnostic
    expect(
      suggestionsUsing(diagnostic, [
        { evidence: [{ session_id: 's', entry_id: 'b' }] },
        { evidence: [{ session_id: 's', entry_id: 'a' }] },
      ]),
    ).toEqual([2])
  })

  it('copies a plan someone can run', () => {
    const markdown = planMarkdown('Scope the notice', {
      scenario_id: null,
      reproduction: 'Change an unrelated function.',
      invariants: ['schedule once'],
      primary_metric: 'redundant lookups',
      expectation: 'fewer lookups',
      non_regression_controls: ['real change still recovers'],
    })
    expect(markdown).toContain('Scenario: new case needed')
    expect(markdown).toContain('- schedule once')
  })
})

describe('briefMarkdown', () => {
  const suggestion: Suggestion = {
    title: 'Scope the notice',
    observation: 'The model called engine::functions::info again.',
    hypothesis: 'A broad notice prompts a new lookup.',
    harness_component: 'Model notices',
    proposed_change: 'Emit the notice only when a function in context changes.',
    expected_effect: 'Fewer repeated lookups.',
    evidence: [{ session_id: 's', entry_id: 'e_t_1_fc_02' }],
    code_refs: [
      { path: 'harness/src/notice.rs', line_from: 120, line_to: 148 },
      { path: 'harness/src/a.rs', line_from: 9, line_to: 9 },
    ],
    limitations: '',
    validation: {
      scenario_id: 'tool_contract_recovery',
      reproduction: 'r',
      invariants: ['schedule once'],
      primary_metric: 'lookups per run',
      expectation: 'fewer',
      non_regression_controls: ['recovery still happens'],
    },
  }
  const input = {
    analysisId: 'eval_1',
    sessionId: 's_root',
    turnId: 't_aaa',
    harnessVersion: '1.8.42',
    index: 0,
    suggestion,
    status: 'Accepted by layon, 02 Oct 21:14',
    criterion: 'lookups per run · lower is better · at least 50 % lower · 5 runs per side',
    codeRoot: '/work/workers',
  }

  it('hands over the stored fields, section by section', () => {
    const brief = briefMarkdown(input)
    expect(brief.split('\n')[0]).toBe('# Implement: Scope the notice')
    expect(brief).toContain('Analysis eval_1 · S1 · observed session s_root · turn t_aaa · Harness 1.8.42')
    expect(brief).toContain('Status: Accepted by layon, 02 Oct 21:14')
    expect(brief).toContain('## Hypothesis (not proven)')
    expect(brief).toContain('Harness area: Model notices')
    expect(brief).toContain('In /work/workers\n- harness/src/notice.rs:120-148\n- harness/src/a.rs:9')
    expect(brief).toContain('e_t_1_fc_02 (open eval_1 for the entries)')
    expect(brief).toContain('Criterion: lookups per run')
    expect(brief).toContain('- schedule once\n- recovery still happens')
  })

  it('leaves out what it has nothing to say about', () => {
    const brief = briefMarkdown({
      ...input,
      status: undefined,
      criterion: undefined,
      codeRoot: undefined,
      harnessVersion: undefined,
      suggestion: { ...suggestion, code_refs: [], evidence: [] },
    })
    expect(brief).not.toContain('## Limitations')
    expect(brief).not.toContain('## Code read')
    expect(brief).not.toContain('## Evidence')
    expect(brief).not.toContain('Status:')
    expect(brief).not.toContain('Criterion:')
    expect(brief).not.toContain('Harness 1')
  })
})

describe('formatStamp', () => {
  it('reads day, month and local time', () => {
    expect(formatStamp(new Date(2026, 9, 3, 10, 12).getTime())).toBe('03 Oct 10:12')
  })
})
