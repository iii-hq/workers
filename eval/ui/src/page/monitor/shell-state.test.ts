import { describe, expect, it } from 'vitest'
import { LIMITS } from '../../fixtures'
import type { AnalysisRecord, MonitorState } from '../../types'
import {
  capacityNotice,
  cardState,
  clock,
  emptyFilterTitle,
  isDeletedAnalysis,
  needsPolling,
  parseOpenContext,
  pausedDetail,
  queueLine,
  REJECTION_WINDOW_MS,
  recentRejection,
  rowMeta,
  rowStatus,
  rowTitle,
  todayLine,
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
  const rejection = { session_id: 'sess_01JA5D0Q', turn_id: 't', at: NOW - 60_000 }

  it('shows within the hour and drops afterwards', () => {
    expect(recentRejection(monitor({ last_rejection: rejection }), NOW)).toEqual(rejection)
    expect(recentRejection(monitor({ last_rejection: rejection }), NOW + REJECTION_WINDOW_MS)).toBeUndefined()
    expect(recentRejection(monitor(), NOW)).toBeUndefined()
    expect(recentRejection(null, NOW)).toBeUndefined()
  })
})

describe('capacityNotice', () => {
  const rejection = { session_id: 'sess_01JA5D0Q', turn_id: 't', at: NOW - 60_000 }
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
