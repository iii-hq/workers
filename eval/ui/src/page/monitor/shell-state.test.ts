import { describe, expect, it } from 'vitest'
import { COST, LIMITS } from '../../fixtures'
import type { AnalysisRecord, MonitorState, ReviewSummary, SuggestionReview } from '../../types'
import {
  capacityNotice,
  cappedNote,
  capRow,
  cardState,
  clock,
  emptyFilterTitle,
  estimateFor,
  estimateLine,
  estimateText,
  filterLabel,
  groupHistory,
  groupSource,
  groupTally,
  indexReviews,
  isDeletedAnalysis,
  memberMeta,
  needsPolling,
  parseOpenContext,
  pausedDetail,
  queueLine,
  REJECTION_WINDOW_MS,
  recentRejection,
  reviewLine,
  reviewMeta,
  rowCause,
  rowMeta,
  rowStatus,
  rowTime,
  rowTitle,
  skippedRow,
  spendDialog,
  TRIAGE_FAILURE_WINDOW_MS,
  todayLine,
  triageNotice,
  triageProblem,
  triageRow,
} from './shell-state'

const NOW = new Date(2026, 9, 2, 20, 0, 0).getTime()
const MORNING = new Date(2026, 9, 2, 8, 30, 0).getTime()
const YESTERDAY = new Date(2026, 9, 1, 22, 0, 0).getTime()

function record(overrides: Partial<AnalysisRecord> = {}): AnalysisRecord {
  return {
    schema_version: 1,
    evaluation_id: 'ev_1',
    observation_key: 'k',
    origin: 'automatic',
    session_id: 'sess_01JA4M7Q',
    turn_id: 't_1',
    model: { model: 'm', provider: 'p' },
    config_revision: 'r',
    rules_version: 'rules',
    criteria_version: 'c',
    status: 'completed',
    step: 0,
    created_at: MORNING,
    updated_at: MORNING,
    deadline: MORNING + 180_000,
    observe_since: MORNING,
    counters: { sessions: 1, entries: 1, diagnostics: 0, suggestions: 0, rejected_suggestions: 0, validations: 0 },
    stages: [],
    usage: { judge_calls: 1, judge_input_tokens: 10, judge_output_tokens: 5, judge_usage_complete: true },
    ...overrides,
  }
}

function monitor(overrides: Partial<MonitorState> = {}): MonitorState {
  return {
    config: {
      enabled: true,
      model: { model: 'claude-sonnet-5-5', provider: 'anthropic' },
      revision: 'rev',
      updated_at: MORNING,
    },
    limits: LIMITS,
    cost: COST,
    observer_bound: true,
    ...overrides,
  }
}

describe('cardState', () => {
  it('waits for the first answer, then follows the configuration', () => {
    expect(cardState(null)).toBe('loading')
    expect(cardState(monitor({ config: null }))).toBe('unconfigured')
    expect(cardState(monitor())).toBe('observing')
  })

  it('reports a paused monitor as paused even if the observer is unbound', () => {
    const config = { ...monitor().config!, enabled: false }
    expect(cardState(monitor({ config }))).toBe('paused')
    expect(cardState(monitor({ config, observer_bound: false }))).toBe('paused')
  })

  it('treats a failed read as unavailable, even with an older answer in hand', () => {
    expect(cardState(null, true)).toBe('unavailable')
    expect(cardState(monitor(), true)).toBe('unavailable')
  })

  it('reports an enabled monitor whose observer did not bind as unavailable', () => {
    expect(cardState(monitor({ observer_bound: false, observer_error: 'nope' }))).toBe('unavailable')
  })

  it('pauses for the day once the cost cap is reached, unless the monitor is paused or unavailable anyway', () => {
    const capped = { ...COST, cap_usd: 5, today_usd: 5.03, capped: true }
    expect(cardState(monitor({ cost: capped }))).toBe('capped')
    expect(cardState(monitor({ cost: { ...capped, capped: false } }))).toBe('observing')
    const paused = { ...monitor().config!, enabled: false }
    expect(cardState(monitor({ cost: capped, config: paused }))).toBe('paused')
    expect(cardState(monitor({ cost: capped, observer_bound: false }))).toBe('unavailable')
  })
})

describe('the daily cost cap on the card', () => {
  const cost = {
    ...COST,
    since: new Date(2026, 9, 3, 21, 0).getTime() - 86_400_000,
    cap_usd: 5,
    today_usd: 5.03,
    capped: true,
  }

  it('says what was reached, what still runs and when new analyses resume', () => {
    expect(cappedNote(cost)).toBe(
      'Cost cap $5.00 reached ($5.03 reported). Manual analyses still run. New ones resume at 21:00.',
    )
  })

  it('shows the Cap row only when a cap is set, and counts the unknown apart', () => {
    expect(capRow({ ...cost, today_usd: 0.91, today_unknown: 2, capped: false })).toBe('$0.91 of $5.00 · 2 unknown')
    expect(capRow({ ...cost, today_usd: 0.91, capped: false })).toBe('$0.91 of $5.00')
    expect(capRow({ ...COST })).toBeNull()
  })

  it('names the last session turned away by the cap, from today only', () => {
    const rejection = { session_id: 's', turn_id: 't', at: cost.since + 3_600_000, reason: 'cost_cap' as const }
    expect(skippedRow(monitor({ cost, last_rejection: rejection }))).toMatch(/^last \d\d:\d\d · cost cap$/)
    expect(skippedRow(monitor({ cost, last_rejection: { ...rejection, at: cost.since - 1 } }))).toBeNull()
    expect(skippedRow(monitor({ cost, last_rejection: { ...rejection, reason: 'at_capacity' } }))).toBeNull()
    expect(skippedRow(monitor({ cost: { ...cost, capped: false }, last_rejection: rejection }))).toBeNull()
  })

  it('does not call a skipped session a full queue', () => {
    const rejection = { session_id: 's', turn_id: 't', at: NOW - 1000, reason: 'cost_cap' as const }
    expect(capacityNotice(monitor({ last_rejection: rejection }), [], NOW)).toBeUndefined()
    expect(capacityNotice(monitor({ last_rejection: { ...rejection, reason: 'at_capacity' } }), [], NOW)).toBeDefined()
  })
})

describe('queue and cost lines', () => {
  it('counts running and pending from the list', () => {
    const records = [
      record({ status: 'investigating' }),
      record({ status: 'judging' }),
      record({ status: 'queued' }),
      record({ status: 'completed' }),
    ]
    expect(queueLine(records, NOW, LIMITS.max_active_analyses)).toBe('2 running · 1 pending · cap 500')
    expect(queueLine(records, NOW, 50)).toBe('2 running · 1 pending · cap 50')
  })

  it('leaves the cap out until the monitor reports it', () => {
    expect(queueLine([record({ status: 'queued' })], NOW, undefined)).toBe('0 running · 1 pending')
  })

  it('never turns an unknown cost into $0', () => {
    const unknown = record({
      usage: {
        judge_calls: 1,
        judge_input_tokens: 1,
        judge_output_tokens: 1,
        judge_usage_complete: true,
        llm_input_tokens: 900,
      },
    })
    expect(todayLine([unknown], NOW)).toBe('1 analysis · cost not reported')
  })

  it('sums the known cost and says how many are not reported', () => {
    const known = record({
      usage: {
        judge_calls: 1,
        judge_input_tokens: 1,
        judge_output_tokens: 1,
        judge_usage_complete: true,
        llm_input_tokens: 10,
        llm_cost_usd: 0.91,
      },
    })
    const unknown = record({
      usage: {
        judge_calls: 1,
        judge_input_tokens: 1,
        judge_output_tokens: 1,
        judge_usage_complete: true,
        llm_input_tokens: 5,
      },
    })
    expect(todayLine([known], NOW)).toBe('1 analysis · $0.91 monitor · Jev not included')
    expect(todayLine([known, unknown], NOW)).toBe('2 analyses · $0.91 monitor · 1 not reported · Jev not included')
  })

  it('only says Jev is left out when Jev was called', () => {
    const noJudge = record({
      usage: {
        judge_calls: 0,
        judge_input_tokens: 0,
        judge_output_tokens: 0,
        judge_usage_complete: false,
        llm_cost_usd: 0.4,
      },
    })
    expect(todayLine([noJudge], NOW)).toBe('1 analysis · $0.40 monitor')
  })

  it('leaves the cost out when nothing reached the analyst model', () => {
    expect(todayLine([record()], NOW)).toBe('1 analysis')
    expect(todayLine([], NOW)).toBe('0 analyses')
  })

  it('counts only today', () => {
    expect(todayLine([record({ created_at: YESTERDAY })], NOW)).toBe('0 analyses')
  })
})

describe('pausedDetail', () => {
  it('says what keeps running', () => {
    expect(pausedDetail(2, 1)).toBe(
      "New sessions aren't analyzed. 2 running analyses continue. Sessions finished while paused can be analyzed by ID.",
    )
    expect(pausedDetail(1, 0)).toContain('1 running analysis continues.')
    expect(pausedDetail(0, 3)).toContain('3 queued analyses continue.')
    expect(pausedDetail(0, 0)).toBe(
      "New sessions aren't analyzed. Sessions finished while paused can be analyzed by ID.",
    )
  })
})

describe('recentRejection', () => {
  const rejection = { session_id: 'sess_01JA5D0Q', turn_id: 't', at: NOW - 60_000, reason: 'at_capacity' as const }

  it('shows within the hour and drops afterwards', () => {
    expect(recentRejection(monitor({ last_rejection: rejection }), NOW)).toEqual(rejection)
    expect(recentRejection(monitor({ last_rejection: rejection }), NOW + REJECTION_WINDOW_MS)).toBeUndefined()
    expect(recentRejection(monitor(), NOW)).toBeUndefined()
    expect(recentRejection(null, NOW)).toBeUndefined()
  })
})

describe('capacityNotice', () => {
  const rejection = { session_id: 'sess_01JA5D0Q', turn_id: 't', at: NOW - 60_000, reason: 'at_capacity' as const }
  const full = Array.from({ length: LIMITS.max_active_analyses }, (_, index) =>
    record({ evaluation_id: `ev_${index}`, status: index % 2 ? 'queued' : 'collecting' }),
  )

  it('is live only while the unfinished analyses still fill the cap', () => {
    expect(capacityNotice(monitor({ last_rejection: rejection }), full, NOW)).toEqual({
      sessionId: 'sess_01JA5D0Q',
      at: rejection.at,
      unfinished: LIMITS.max_active_analyses,
      live: true,
    })
    expect(capacityNotice(monitor({ last_rejection: rejection }), [record()], NOW)).toMatchObject({
      unfinished: 0,
      live: false,
    })
    expect(capacityNotice(monitor({ last_rejection: rejection }), null, NOW)?.live).toBe(false)
  })

  it('reads the cap from the monitor state, not from a constant', () => {
    const small = monitor({ last_rejection: rejection, limits: { ...LIMITS, max_active_analyses: 3 } })
    const three = full.slice(0, 3)
    expect(capacityNotice(small, three, NOW)).toMatchObject({ unfinished: 3, live: true })
    expect(capacityNotice(small, three.slice(0, 2), NOW)).toMatchObject({ unfinished: 2, live: false })
    // 100 unfinished fill nothing when the cap is 500.
    expect(capacityNotice(monitor({ last_rejection: rejection }), full.slice(0, 100), NOW)?.live).toBe(false)
  })

  it('is absent without a recent rejection', () => {
    expect(capacityNotice(monitor(), full, NOW)).toBeUndefined()
    expect(capacityNotice(monitor({ last_rejection: rejection }), full, NOW + REJECTION_WINDOW_MS)).toBeUndefined()
  })
})

describe('needsPolling', () => {
  it('polls only while an analysis is unfinished', () => {
    expect(needsPolling(null)).toBe(false)
    expect(needsPolling([record()])).toBe(false)
    expect(needsPolling([record({ status: 'queued' })])).toBe(true)
    expect(needsPolling([record({ status: 'failed' }), record({ status: 'cancelled' })])).toBe(false)
  })

  it('also polls while the observer has not bound yet', () => {
    expect(needsPolling([record()], monitor({ observer_bound: false }))).toBe(true)
    expect(needsPolling(null, monitor({ observer_bound: false }))).toBe(true)
    expect(needsPolling([record()], monitor())).toBe(false)
    expect(needsPolling([record()], null)).toBe(false)
  })
})

describe('isDeletedAnalysis', () => {
  it('recognizes the conflict of a turn whose analysis was deleted', () => {
    expect(
      isDeletedAnalysis(
        'evaluation conflict: the analysis ev_1 of this turn was deleted; set reanalyze to create another',
      ),
    ).toBe(true)
    expect(isDeletedAnalysis('evaluation conflict: the session has not finished')).toBe(false)
  })
})

describe('rows', () => {
  it('formats the clock in local time', () => {
    expect(clock(new Date(2026, 9, 2, 19, 41).getTime())).toBe('19:41')
    expect(clock(new Date(2026, 9, 2, 7, 5).getTime())).toBe('07:05')
  })

  it('reads quieter when a finished analysis has no suggestions', () => {
    expect(rowStatus(record()).quiet).toBe(true)
    const withSuggestion = record({ counters: { ...record().counters, suggestions: 1 } })
    expect(rowStatus(withSuggestion)).toMatchObject({ label: '1 suggestion', tone: 'ok', quiet: false })
    expect(
      rowStatus(record({ status: 'failed', failure: { stage: 'judging', code: 'judge_bus', message: 'x' } })).tone,
    ).toBe('alert')
  })

  it('says which limit an over-size capture went over', () => {
    const over = record({
      status: 'failed',
      failure: {
        stage: 'collecting',
        code: 'coverage_insufficient',
        message: 'captured evidence (421888 bytes) exceeds the 262144-byte limit; no model was called',
      },
    })
    expect(rowMeta(over)).toBe('over 256 KiB')
    expect(rowMeta({ ...over, failure: { ...over.failure!, message: 'unparsable' } })).toBe('over the evidence limit')
    expect(rowMeta(record())).toBe('no signals')
  })

  it('titles a row by the source title, else the session id', () => {
    expect(rowTitle(record({ source_title: '  Fix the flaky test ' }))).toBe('Fix the flaky test')
    expect(rowTitle(record({ source_title: '   ' }))).toBe('sess_01JA4M7Q')
    expect(rowTitle(record())).toBe('sess_01JA4M7Q')
  })

  it('names the empty filters', () => {
    expect(emptyFilterTitle('failed')).toBe('No failed analyses')
    expect(emptyFilterTitle('suggestions')).toBe('No analyses with suggestions')
  })
})

describe('parseOpenContext', () => {
  it('reads the two contexts the page is opened with', () => {
    expect(parseOpenContext(3, { type: 'analysis', evaluationId: 'ev_1' })).toEqual({
      seq: 3,
      type: 'analysis',
      evaluationId: 'ev_1',
    })
    expect(parseOpenContext(4, { type: 'analyze' })).toEqual({ seq: 4, type: 'analyze' })
  })

  it('ignores anything else, including the old prompt-experiment contexts', () => {
    expect(parseOpenContext(1, { type: 'analysis' })).toBeNull()
    expect(parseOpenContext(1, { type: 'new' })).toBeNull()
    expect(parseOpenContext(1, { type: 'evaluation', evaluationId: 'ev_1' })).toBeNull()
    expect(parseOpenContext(1, null)).toBeNull()
    expect(parseOpenContext(1, 'analysis')).toBeNull()
  })
})

describe('whether triage can run', () => {
  const failedCredits = (at: number, extra: Partial<AnalysisRecord> = {}) =>
    record({
      status: 'failed',
      created_at: at - 1000,
      completed_at: at,
      failure: { stage: 'judging', code: 'judge_out_of_credits', message: 'm' },
      ...extra,
    })
  const answered = (at: number) =>
    record({ created_at: at - 1000, completed_at: at, routing: { investigate: true, reasons: ['diagnostics'] } })

  it('believes the provider check when it says no', () => {
    const triage = {
      provider: 'judge-typesafe',
      available: false,
      code: 'missing_key',
      models: [],
      checked_at: NOW - 5000,
    }
    expect(triageProblem(triage, [], NOW)).toEqual({ code: 'missing_key', at: NOW - 5000, source: 'check' })
    expect(triageProblem({ ...triage, code: undefined }, [], NOW)?.code).toBe('unreachable')
  })

  it('knows an HTTP 402 from the analyses, since the check lists models and cannot see credits', () => {
    const triage = { provider: 'judge-typesafe', available: true, models: ['jev'], checked_at: NOW }
    expect(triageProblem(triage, [failedCredits(NOW - 60_000)], NOW)).toEqual({
      code: 'out_of_credits',
      at: NOW - 60_000,
      source: 'analysis',
    })
    // The same with no check yet.
    expect(triageProblem(undefined, [failedCredits(NOW - 60_000)], NOW)?.code).toBe('out_of_credits')
  })

  it('forgets a 402 once a later analysis got an answer, or after the hour', () => {
    expect(triageProblem(undefined, [failedCredits(NOW - 60_000), answered(NOW - 30_000)], NOW)).toBeUndefined()
    // An answer older than the failure does not clear it.
    expect(triageProblem(undefined, [answered(NOW - 90_000), failedCredits(NOW - 60_000)], NOW)?.code).toBe(
      'out_of_credits',
    )
    expect(triageProblem(undefined, [failedCredits(NOW - TRIAGE_FAILURE_WINDOW_MS)], NOW)).toBeUndefined()
    expect(triageProblem(undefined, null, NOW)).toBeUndefined()
  })

  it('counts an old `judge_http` that says 402 as no credits too', () => {
    const old = record({
      status: 'failed',
      created_at: NOW - 61_000,
      completed_at: NOW - 60_000,
      failure: { stage: 'judging', code: 'judge_http', message: 'judge-typesafe answered (HTTP 402): no credits' },
    })
    expect(triageProblem(undefined, [old], NOW)?.code).toBe('out_of_credits')
  })

  it('ignores other failures: they say nothing about credits', () => {
    const other = record({ status: 'failed', failure: { stage: 'judging', code: 'judge_deadline', message: 'm' } })
    expect(triageProblem(undefined, [other], NOW)).toBeUndefined()
  })

  it('writes the Triage row and the warning', () => {
    const credits = { code: 'out_of_credits', at: new Date(2026, 9, 2, 14, 2).getTime(), source: 'analysis' as const }
    const key = { code: 'missing_key', at: credits.at, source: 'check' as const }
    expect(triageRow(credits, true, 'judge-typesafe')).toBe('unavailable · HTTP 402')
    expect(triageRow(key, true, 'judge-typesafe')).toBe('unavailable · missing key')
    expect(triageRow(undefined, true, 'judge-typesafe')).toBe('judge-typesafe · available')
    expect(triageRow(undefined, false, 'judge-typesafe')).toBe('judge-typesafe')
    expect(triageNotice(credits)).toBe(
      'Jev: out of credits (HTTP 402), last triage 14:02. New analyses will fail at triage.',
    )
    expect(triageNotice(key)).toBe('Jev: no API key, checked 14:02. New analyses will fail at triage.')
  })
})

describe('what an analysis costs', () => {
  const state = (stats: MonitorState['cost']['per_analysis'], codeRepository?: string) =>
    monitor({
      cost: { ...COST, per_analysis: stats },
      config: { ...monitor().config!, ...(codeRepository ? { code_repository: codeRepository } : {}) },
    })
  const stats = { count: 9, min: 0.04, median: 0.38, max: 1.27, unknown: 0 }
  const ran = (minutes: number, overrides: Partial<AnalysisRecord> = {}) =>
    record({
      created_at: MORNING,
      completed_at: MORNING + minutes * 60_000,
      analyst: { session_id: 'a', sent_at: MORNING },
      model: { model: 'claude-sonnet-5-5', provider: 'anthropic' },
      ...overrides,
    })

  it('needs three analyses with a cost before it says a figure', () => {
    expect(estimateFor(state({ count: 2, min: 0.1, median: 0.2, max: 0.3, unknown: 0 }), [])).toEqual({ count: 2 })
    expect(estimateFor(state({ count: 0, unknown: 4 }), [])).toEqual({ count: 0 })
    expect(estimateFor(null, [])).toEqual({ count: 0 })
    expect(estimateFor(state(stats), [])).toEqual({ count: 9, cost: [0.38, 1.27], time: undefined })
  })

  it('takes the time from the listed analyses of the same setup, from the median to the longest', () => {
    const records = [
      ran(2),
      ran(4),
      ran(6),
      ran(30, { model: { model: 'other', provider: 'anthropic' } }),
      ran(40, { code_root: '/c' }),
    ]
    expect(estimateFor(state(stats), records).time).toEqual([240_000, 360_000])
    // Not enough of them listed: no time, but the cost stays.
    expect(estimateFor(state(stats), [ran(2), ran(4)]).time).toBeUndefined()
  })

  it('writes a range, and one figure when the analyses all cost the same', () => {
    expect(estimateText({ count: 9, cost: [0.38, 1.27], time: [120_000, 360_000] })).toBe('≈ $0.38–1.27, 2–6 min')
    expect(estimateText({ count: 9, cost: [0.38, 1.27] })).toBe('≈ $0.38–1.27')
    expect(estimateText({ count: 3, cost: [0.5, 0.5] })).toBe('≈ $0.50')
    expect(estimateText({ count: 1 })).toBeNull()
  })

  it('ends a failure with the estimate, or with why there is none', () => {
    expect(estimateLine({ count: 9, cost: [0.38, 1.27], time: [120_000, 360_000] })).toBe(
      'Reanalyze runs the whole pipeline again: ≈ $0.38–1.27, 2–6 min, based on 9 analyses with this model.',
    )
    expect(estimateLine({ count: 2 })).toBe('No estimate yet: fewer than 3 completed analyses with this model.')
  })

  const base = {
    config: {
      ...monitor().config!,
      model: { model: 'claude-opus-5-5', provider: 'anthropic', thinking_level: 'xhigh' as const },
      code_repository: '/home/layon/workspaces/workers',
    },
    limits: LIMITS,
    problem: undefined,
    now: NOW,
  }

  it('asks before a reanalysis, with the estimate when there is one', () => {
    const dialog = spendDialog({
      ...base,
      kind: 'reanalyze',
      estimate: { count: 9, cost: [0.38, 1.27], time: [120_000, 360_000] },
    })
    expect(dialog).toEqual({
      title: 'Reanalyze this session?',
      description:
        'The whole pipeline runs again: collection, triage, and investigation if Jev answers needs_investigation with claude-opus-5-5 · xhigh, reading code in /home/layon/workspaces/workers.',
      details: [
        'Estimate ≈ $0.38–1.27, 2–6 min',
        'Based on 9 completed analyses with this model and code access',
        'The earlier analysis stays in the list; the new one is linked to it as its replacement',
      ],
      confirmLabel: 'Reanalyze',
      recheck: false,
    })
  })

  it('says there is no estimate, and what bounds one analysis, instead of guessing', () => {
    const dialog = spendDialog({ ...base, kind: 'reanalyze', estimate: { count: 1 } })
    expect(dialog?.details).toEqual([
      'No estimate yet: fewer than 3 completed analyses with this model and code access',
      'Each analysis is capped at 32 steps and 800k tokens',
      'Cost is shown on the analysis once it ends, never guessed',
    ])
  })

  it('asks before Analyze only when it has a figure to show', () => {
    expect(spendDialog({ ...base, kind: 'analyze', estimate: { count: 1 } })).toBeNull()
    const dialog = spendDialog({ ...base, kind: 'analyze', estimate: { count: 9, cost: [0.38, 1.27] } })
    expect(dialog?.title).toBe('Analyze this session?')
    expect(dialog?.confirmLabel).toBe('Analyze')
    expect(dialog?.details).not.toContain(
      'The earlier analysis stays in the list; the new one is linked to it as its replacement',
    )
  })

  it('does not ask before starting observation unless triage cannot run', () => {
    expect(spendDialog({ ...base, kind: 'start', estimate: { count: 9, cost: [0.38, 1.27] } })).toBeNull()
    expect(spendDialog({ ...base, kind: 'resume', estimate: { count: 9, cost: [0.38, 1.27] } })).toBeNull()
  })

  it('puts triage that cannot run first, and offers to check again when only the provider check can tell', () => {
    const credits = { code: 'out_of_credits', at: NOW - 120_000, source: 'analysis' as const }
    const estimate = { count: 9, cost: [0.38, 1.27] as [number, number], time: [120_000, 360_000] as [number, number] }
    const dialog = spendDialog({ ...base, kind: 'reanalyze', estimate, problem: credits })
    expect(dialog).toEqual({
      title: 'Triage is unavailable',
      description:
        'Jev answered HTTP 402 (no credits) on its last triage, 2 min ago. A new analysis would fail at triage again.',
      details: [
        'Nothing is spent on the investigation when triage fails',
        'Add credits in TypeSafe billing, then analyze again',
        'Estimate if it worked: ≈ $0.38–1.27, 2–6 min',
      ],
      confirmLabel: 'Reanalyze anyway',
      // The provider check lists models: it cannot see credits come back.
      recheck: false,
    })
    const key = spendDialog({
      ...base,
      kind: 'start',
      estimate: { count: 0 },
      problem: { code: 'missing_key', at: NOW - 120_000, source: 'check' },
    })
    expect(key?.description).toBe(
      "Jev can't run triage: no API key, checked 2 min ago. A new analysis would fail at triage again.",
    )
    expect(key?.details).toEqual([
      'Nothing is spent on the investigation when triage fails',
      'Add the key in judge-typesafe settings.',
    ])
    expect(key?.confirmLabel).toBe('Save and start anyway')
    expect(key?.recheck).toBe(true)
    expect(
      spendDialog({
        ...base,
        kind: 'resume',
        estimate: { count: 0 },
        problem: { code: 'missing_key', at: NOW, source: 'check' },
      })?.confirmLabel,
    ).toBe('Resume anyway')
  })
})

describe('the history grouped by turn', () => {
  const at = (hours: number) => new Date(2026, 9, 2, 8 + hours, 0).getTime()
  const turn = (id: string, key: string, hours: number, overrides: Partial<AnalysisRecord> = {}) =>
    record({ evaluation_id: id, observation_key: key, created_at: at(hours), updated_at: at(hours), ...overrides })
  const withSuggestions = { counters: { ...record().counters, suggestions: 2 } }
  const failed = {
    status: 'failed' as const,
    failure: { stage: 'investigating' as const, code: 'analyst_failed', message: 'm' },
  }

  it('groups two or more analyses of one turn and leaves a single one a plain row', () => {
    const items = groupHistory(
      [turn('a1', 'k1', 1), turn('solo', 'k2', 2), turn('a2', 'k1', 3), turn('a3', 'k1', 0)],
      'all',
    )
    expect(items.map((item) => (item.kind === 'row' ? item.record.evaluation_id : item.key))).toEqual(['k1', 'solo'])
    const group = items[0]
    expect(group.kind === 'group' && group.members.map((member) => member.evaluation_id)).toEqual(['a2', 'a1', 'a3'])
  })

  it('orders items by their newest member, a group among the plain rows', () => {
    const items = groupHistory(
      [turn('old', 'k1', 0), turn('mid', 'k2', 2), turn('new', 'k1', 4), turn('last', 'k3', 5)],
      'all',
    )
    expect(items.map((item) => (item.kind === 'row' ? item.record.evaluation_id : item.key))).toEqual([
      'last',
      'k1',
      'mid',
    ])
  })

  it('lets the filter narrow the rows, drops a group with none left and still tallies every member', () => {
    const records = [
      turn('a', 'k1', 1, withSuggestions),
      turn('b', 'k1', 2, failed),
      turn('c', 'k1', 3, failed),
      turn('d', 'k2', 4, failed),
      turn('e', 'k2', 5, failed),
    ]
    const suggestions = groupHistory(records, 'suggestions')
    expect(suggestions).toHaveLength(1)
    const [group] = suggestions
    if (group.kind !== 'group') throw new Error('group expected')
    expect(group.shown.map((member) => member.evaluation_id)).toEqual(['a'])
    expect(groupTally(group.members)).toBe('3 analyses: 1 with suggestions, 2 failed')
    expect(groupHistory(records, 'failed')).toHaveLength(2)
    expect(groupHistory(records, 'active')).toEqual([])
  })

  it('tallies each kind of outcome, and says nothing about the kinds that did not happen', () => {
    const members = [
      turn('1', 'k', 1, withSuggestions),
      turn('2', 'k', 2),
      turn('3', 'k', 3, { status: 'cancelled' }),
      turn('4', 'k', 4, { status: 'investigating' }),
    ]
    expect(groupTally(members)).toBe('4 analyses: 1 with suggestions, 1 without suggestions, 1 cancelled, 1 running')
  })

  it('names the turn a group observed', () => {
    expect(groupSource(record({ turn_id: 't_e864a1b2c3d4e5f6', session_id: 'e2e_36328da3' }))).toBe(
      'turn t_e864… · e2e_36328da3',
    )
  })

  it('says a failure in words, and a 402 with its status', () => {
    expect(rowCause(record(failed))).toBe('The analyst ended without a result')
    expect(
      rowCause(record({ status: 'failed', failure: { stage: 'judging', code: 'judge_out_of_credits', message: 'm' } })),
    ).toBe('Jev is out of credits (402)')
    // Stored before the code existed: a `judge_http` that says 402.
    expect(
      rowCause(
        record({
          status: 'failed',
          failure: { stage: 'judging', code: 'judge_http', message: 'judge-typesafe answered (HTTP 402): no credits' },
        }),
      ),
    ).toBe('Jev is out of credits (402)')
    expect(rowCause(record())).toBeUndefined()
  })

  it('says what an analysis of a group cost and took, and never a cost it did not report', () => {
    const analyst = { session_id: 'a', sent_at: 0 }
    expect(
      memberMeta(
        record({
          created_at: MORNING,
          completed_at: MORNING + 380_000,
          code_root: '/c',
          analyst,
          usage: { ...record().usage, llm_cost_usd: 1.27 },
        }),
      ),
    ).toBe('$1.27 · 6m 20s · reads code')
    expect(memberMeta(record({ created_at: MORNING, completed_at: MORNING + 52_000, analyst }))).toBe(
      'no cost reported · 52 s · no code',
    )
    // Never investigated: there is no cost to report.
    expect(memberMeta(record({ created_at: MORNING, completed_at: MORNING + 5000 }))).toBe('5 s · no code')
    expect(memberMeta(record({ created_at: MORNING, status: 'investigating', analyst }))).toBe(
      'no cost reported · no code',
    )
  })

  it('puts the day in the time of a row', () => {
    expect(rowTime(record({ created_at: MORNING }), NOW)).toBe('Today 08:30')
    expect(rowTime(record({ created_at: YESTERDAY }), NOW)).toBe('Yesterday 22:00')
  })
})

describe('what people decided, in the list', () => {
  const summary = (evaluation_id: string, counts: Partial<ReviewSummary>): ReviewSummary => ({
    evaluation_id,
    suggestions: 0,
    new: 0,
    accepted: 0,
    in_progress: 0,
    shipped: 0,
    rejected: 0,
    duplicate: 0,
    ...counts,
  })
  const row = (evaluation_id: string, suggestion_index: number, status: SuggestionReview['lifecycle']['status']) =>
    ({ evaluation_id, suggestion_index, lifecycle: { status, history: [] } }) as unknown as SuggestionReview
  const index = indexReviews({
    summaries: [
      summary('a', { suggestions: 1, new: 1 }),
      summary('b', { suggestions: 2, new: 1, in_progress: 1 }),
      summary('c', { suggestions: 1, shipped: 1 }),
      summary('d', { suggestions: 2, shipped: 1, rejected: 1 }),
    ],
    reviews: [row('b', 0, 'in_progress'), row('c', 0, 'shipped'), row('d', 0, 'shipped'), row('d', 1, 'rejected')],
  })
  const records = ['a', 'b', 'c', 'd', 'e'].map((id) =>
    record({ evaluation_id: id, observation_key: id, counters: { ...record().counters, suggestions: 1 } }),
  )

  it('reads nothing before the counts, or from a worker that predates them', () => {
    expect(indexReviews(null)).toBeNull()
    expect(reviewLine(records[0], null)).toBeUndefined()
    expect(reviewMeta(records[1], null)).toBeUndefined()
    expect(filterLabel('suggestions', 'Suggestions', records, null)).toBe('Suggestions')
  })

  it('says where the suggestions of an analysis stand', () => {
    expect(reviewLine(records[0], index)).toBe('1 suggestion · new')
    expect(reviewLine(records[1], index)).toBe('2 suggestions · 1 new')
    expect(reviewLine(records[2], index)).toBe('1 suggestion · shipped')
    expect(reviewLine(records[3], index)).toBe('2 suggestions · all reviewed')
    // No counts for it (no suggestions): the monitor's own words stay.
    expect(reviewLine(records[4], index)).toBeUndefined()
  })

  it('lists each suggestion once somebody has acted on one, an untouched one as new', () => {
    expect(reviewMeta(records[0], index)).toBeUndefined()
    expect(reviewMeta(records[1], index)).toBe('S1 in progress · S2 new')
    expect(reviewMeta(records[2], index)).toBe('S1 shipped')
    expect(reviewMeta(records[3], index)).toBe('S1 shipped · S2 rejected')
  })

  it('counts the analyses with a suggestion still new under "To review"', () => {
    expect(filterLabel('suggestions', 'Suggestions', records, index)).toBe('To review 2')
    expect(filterLabel('all', 'All', records, index)).toBe('All')
    expect(filterLabel('suggestions', 'Suggestions', [records[2]], index)).toBe('To review')
  })

  it('filters by what is left to review, not by whether suggestions exist', () => {
    const shown = (filter: 'suggestions' | 'all') =>
      groupHistory(records, filter, index).map((item) => (item.kind === 'row' ? item.record.evaluation_id : item.key))
    expect(shown('suggestions').sort()).toEqual(['a', 'b'])
    expect(shown('all')).toHaveLength(5)
    // Without the counts it is the old filter: every analysis with suggestions.
    expect(groupHistory(records, 'suggestions')).toHaveLength(5)
    expect(emptyFilterTitle('suggestions', true)).toBe('Nothing to review')
  })
})
