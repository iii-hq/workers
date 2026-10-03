import { describe, expect, it } from 'vitest'
import { LIMITS } from '../../../fixtures'
import type { AnalysisAssets, AnalysisRecord, AnalysisResult, Snapshot, Triage } from '../../../types'
import { canReanalyze, describeState, usageSummary } from './notices'
import {
  choiceBars,
  clock,
  codeRefAria,
  codeRefCopy,
  codeRefLabel,
  formatCostShort,
  investigationCaps,
  locateEntry,
  metaRest,
  parseSizes,
  pathSegments,
  probeParts,
  routingSentence,
  seconds,
  shortId,
  signalFooter,
} from './present'

const T0 = new Date(2026, 9, 2, 19, 24, 0).getTime()

function record(overrides: Partial<AnalysisRecord> = {}): AnalysisRecord {
  return {
    schema_version: 1,
    evaluation_id: 'eval_1',
    observation_key: 'k',
    origin: 'automatic',
    session_id: 's_root',
    turn_id: 't_0123456789abcdef0123456789abcdef',
    model: { model: 'claude-sonnet-5-5', provider: 'anthropic' },
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

function triage(choice: string, confidence: number): Triage {
  return {
    provider: 'typesafe',
    request_id: 'req',
    model: 'jev',
    criteria_version: 'c',
    answers: {
      investigation: {
        type: 'choice',
        choice,
        confidence,
        probabilities: { expected_behavior: 0.08, needs_investigation: 0.71, insufficient_evidence: 0.21 },
      },
    },
    stats: {
      attempts: 1,
      requests: 1,
      questions: 1,
      input_tokens: 1,
      output_tokens: 1,
      elapsed_ms: 1,
      usage_complete: true,
    },
    completed_at: T0,
  }
}

function result(rec: AnalysisRecord, assets: Partial<AnalysisAssets> = {}): AnalysisResult {
  return { record: rec, assets: { evaluation_id: rec.evaluation_id, validations: [], ...assets } }
}

const failed = (code: string, stage: AnalysisRecord['status'], message = 'm', extra: Partial<AnalysisRecord> = {}) =>
  result(record({ status: 'failed', completed_at: T0 + 1000, failure: { code, stage, message }, ...extra }))

describe('describeState', () => {
  it('has no notice when suggestions are the story', () => {
    const done = record({ status: 'completed', counters: { ...record().counters, suggestions: 1 } })
    expect(describeState(result(done), T0, LIMITS)).toBeNull()
    // Every proposal rejected: the Suggestions section explains it.
    const rejected = record({ status: 'completed', counters: { ...record().counters, rejected_suggestions: 2 } })
    expect(describeState(result(rejected), T0, LIMITS)).toBeNull()
  })

  it('says how long a waiting collection has left', () => {
    const waiting = record({
      status: 'collecting',
      pending_reason: 'descendant sessions are still running or their metrics are incomplete',
    })
    const copy = describeState(result(waiting), T0 + 60_000, LIMITS)
    expect(copy?.tone).toBe('warn')
    expect(copy?.title).toBe('Waiting for the session tree to finish')
    expect(copy?.body).toContain('2m 00s left')
    expect(copy?.actions).toEqual(['cancel'])
  })

  it('names the Jev call while triage runs', () => {
    const judging = record({
      status: 'judging',
      counters: { ...record().counters, diagnostics: 2 },
      judge_call: { request_id: 'eval_1-judge-1', started_at: T0 + 5000, deadline: T0 + 180_000 },
    })
    const copy = describeState(result(judging), T0 + 6000, LIMITS)
    expect(copy?.body).toBe(`2 signals recorded. The call started at ${clock(T0 + 5000)} and times out after 60 s.`)
    expect(copy?.detail).toBe('request eval_1-judge-1')
  })

  it('explains an investigation in flight with the routing reason', () => {
    const investigating = record({
      status: 'investigating',
      counters: { ...record().counters, diagnostics: 1 },
      routing: { investigate: true, reasons: ['diagnostics', 'low_confidence'] },
      analyst: { session_id: 'eval_monitor_eval_1', sent_at: T0 },
    })
    const copy = describeState(result(investigating, { triage: triage('needs_investigation', 0.64) }), T0, LIMITS)
    expect(copy?.body).toBe(
      'Sent because a rule found a signal, and confidence 0.64 is below 0.80. 1 step at most, up to 200,000 tokens.',
    )
    expect(copy?.actions).toEqual(['cancel', 'session'])
    expect(copy?.detail).toBe('claude-sonnet-5-5 · anthropic')
  })

  it('names the caps of the analysis, higher when it was admitted with a code directory', () => {
    const investigating = record({
      status: 'investigating',
      code_root: '/home/layon/workspaces/workers',
      routing: { investigate: true, reasons: ['manual_request'] },
    })
    expect(describeState(result(investigating), T0, LIMITS)?.body).toBe(
      'Sent because it was requested manually. 32 steps at most, up to 800,000 tokens.',
    )
  })

  it('takes every limit it names from the monitor, and names none before they are read', () => {
    const judging = record({
      status: 'judging',
      judge_call: { request_id: 'r', started_at: T0 + 5000, deadline: T0 + 180_000 },
    })
    const slow = { ...LIMITS, judge_timeout_ms: 90_000 }
    expect(describeState(result(judging), T0, slow)?.body).toContain('times out after 90 s.')
    expect(describeState(result(judging), T0, undefined)?.body).toBe(
      `No signals recorded. The call started at ${clock(T0 + 5000)}.`,
    )

    const investigating = record({
      status: 'investigating',
      routing: { investigate: true, reasons: ['low_confidence'] },
    })
    const wide = { ...LIMITS, investigation_max_turns: 2, investigation_max_total_tokens: 50_000, low_confidence: 0.65 }
    expect(describeState(result(investigating, { triage: triage('expected_behavior', 0.6) }), T0, wide)?.body).toBe(
      'Sent because confidence 0.60 is below 0.65. 2 steps at most, up to 50,000 tokens.',
    )
    expect(
      describeState(result(investigating, { triage: triage('expected_behavior', 0.6) }), T0, undefined)?.body,
    ).toBe('Sent because confidence 0.60 is below its threshold. The analyst works within its step and token budget.')

    const deadline = failed('judge_deadline', 'judging')
    expect(describeState(deadline, T0, slow)?.body).toContain("Jev didn't answer within its 90 s limit.")
    expect(describeState(deadline, T0, undefined)?.body).toContain("Jev didn't answer within its time limit.")
  })

  it('reports a quiet completion without calling the task healthy', () => {
    const quiet = record({
      status: 'completed',
      coverage: 'complete',
      routing: { investigate: true, reasons: ['audit_sample'] },
    })
    const copy = describeState(result(quiet, { triage: triage('expected_behavior', 0.92) }), T0, LIMITS)
    expect(copy?.title).toBe('Nothing worth changing was found')
    expect(copy?.tone).toBe('info')
    expect(copy?.body).toBe(
      'No signals. Triage said expected_behavior at 0.92. The session was investigated as part of the audit sample.',
    )
  })

  it('never reads a partial or insufficient capture as a clean bill of health', () => {
    for (const coverage of ['partial', 'insufficient'] as const) {
      const done = record({ status: 'completed', coverage, routing: { investigate: false, reasons: [] } })
      const copy = describeState(result(done), T0, LIMITS)
      expect(copy?.title, coverage).toBe('No suggestions were made')
      expect(copy?.icon, coverage).not.toBe('check')
      expect(copy?.body, coverage).toContain(
        `The capture was ${coverage}, so this doesn't mean the session was healthy.`,
      )
    }
    const insufficient = record({ status: 'completed', coverage: 'insufficient' })
    expect(describeState(result(insufficient), T0, LIMITS)?.tone).toBe('warn')
    expect(describeState(result(record({ status: 'completed', coverage: 'partial' })), T0, LIMITS)?.tone).toBe('info')
    // Without a recorded coverage nothing is claimed either.
    expect(describeState(result(record({ status: 'completed' })), T0, LIMITS)?.title).toBe('No suggestions were made')
  })

  it('gives every failure a precise title and keeps the code', () => {
    const cases: Array<[string, AnalysisRecord['status'], string]> = [
      ['judge_provider_unavailable', 'judging', 'judge-typesafe is unavailable'],
      ['judge_missing_key', 'judging', 'judge-typesafe has no API key'],
      ['judge_deadline', 'judging', 'Triage ran out of time'],
      ['judge_bus', 'judging', "Triage couldn't reach judge-typesafe"],
      ['judge_invalid_response', 'judging', 'Triage returned an unusable answer'],
      ['judge_cancelled', 'judging', 'Triage was cancelled'],
      ['judge_something_new', 'judging', 'Triage failed'],
      ['deadline', 'investigating', 'The analysis ran out of time'],
      ['source_not_found', 'collecting', 'The session has no turn record'],
      ['source_advanced', 'collecting', 'The session changed during capture'],
      ['inconsistent_evidence', 'collecting', 'The session changed during capture'],
      ['evidence_unreadable', 'collecting', "The evidence couldn't be read"],
      ['not_a_root_session', 'collecting', 'This is not a root session'],
      ['monitor_session', 'collecting', "The monitor doesn't analyze its own sessions"],
      ['snapshot_missing', 'judging', 'The saved capture is missing'],
      ['external_outcome_unknown', 'judging', 'Triage result is unknown'],
      ['external_outcome_unknown', 'investigating', 'Investigation result is unknown'],
      ['analyst_rejected', 'investigating', "Harness didn't accept the investigation"],
      ['analyst_failed', 'investigating', 'The investigation turn failed'],
      ['analyst_turn_changed', 'investigating', 'The investigation session changed'],
      ['analyst_step_cap', 'investigating', 'The investigation ran out of steps'],
      ['analyst_output_invalid', 'investigating', "The analyst's answer was unusable"],
      ['brand_new_code', 'collecting', 'The analysis failed'],
    ]
    for (const [code, stage, title] of cases) {
      const copy = describeState(failed(code, stage), T0, LIMITS)
      expect(copy?.title, code).toBe(title)
      expect(copy?.detail, code).toContain(code)
      expect(copy?.tone, code).not.toBe('info')
      // A new run would hit the same refusal: no Reanalyze for these two.
      const again = code !== 'not_a_root_session' && code !== 'monitor_session'
      expect(copy?.actions.includes('reanalyze'), code).toBe(again)
      expect(canReanalyze(failed(code, stage).record), code).toBe(again)
    }
  })

  it('tells an unknown outcome apart from a restart it cannot repeat', () => {
    const copy = describeState(
      failed('external_outcome_unknown', 'judging', 'm', {
        judge_call: { request_id: 'eval-ev_93ad1f60', started_at: T0, deadline: T0 + 1 },
      }),
      T0,
      LIMITS,
    )
    expect(copy?.body).toContain("The call wasn't repeated, so it can't be charged twice.")
    expect(copy?.detail).toContain('request eval-ev_93ad1f60')
  })

  it('reads the sizes of an evidence that is too large', () => {
    const message = 'captured evidence (421888 bytes) exceeds the 262144-byte limit; no model was called'
    const copy = describeState(
      failed('coverage_insufficient', 'collecting', message, {
        counters: { ...record().counters, diagnostics: 1 },
      }),
      T0,
      LIMITS,
    )
    expect(copy?.tone).toBe('warn')
    expect(copy?.body).toContain('The snapshot needs 412.0 KiB; the limit is 256.0 KiB.')
    expect(copy?.body).toContain("This doesn't mean the session was healthy.")
    expect(copy?.actions).toEqual(['signals', 'reanalyze'])
    expect(parseSizes('nothing here')).toBeUndefined()
  })

  it('keeps usage on a cancellation, and unknown cost stays unknown', () => {
    const cancelled = record({
      status: 'cancelled',
      completed_at: T0 + 9000,
      stages: [
        { status: 'queued', at: T0 },
        { status: 'investigating', at: T0 + 4000 },
        { status: 'cancelled', at: T0 + 9000 },
      ],
      usage: {
        judge_calls: 1,
        judge_input_tokens: 1000,
        judge_output_tokens: 204,
        judge_usage_complete: true,
        llm_input_tokens: 8000,
        llm_output_tokens: 208,
      },
    })
    const copy = describeState(result(cancelled), T0, LIMITS)
    expect(copy?.title).toBe('Cancelled during the investigation')
    expect(copy?.detail).toBe('9,412 tokens · cost not reported')
    expect(usageSummary(record().usage)).toBe('No model was called')
    expect(usageSummary({ ...cancelled.usage, llm_cost_usd: 0.0414 })).toBe('9,412 tokens · $0.0414 LLM')
  })
})

describe('present', () => {
  it('rounds a total to the cent, keeps tiny costs precise and unknown costs unknown', () => {
    expect(formatCostShort(0.91)).toBe('$0.91')
    expect(formatCostShort(0.21)).toBe('$0.21')
    expect(formatCostShort(3.456)).toBe('$3.46')
    expect(formatCostShort(0)).toBe('$0.00')
    expect(formatCostShort(0.0041)).toBe('$0.0041')
    expect(formatCostShort(undefined)).toBe('not reported')
  })

  it('formats seconds the way the pipeline reads', () => {
    expect(seconds(200)).toBe('0.2 s')
    expect(seconds(47_300)).toBe('47.3 s')
    expect(seconds(180_000, 0)).toBe('180 s')
    expect(shortId('t_0123456789abcdef0123456789abcdef')).toBe('t_01234567…')
    expect(shortId('t_7')).toBe('t_7')
  })

  it('writes the routing sentence from the reasons', () => {
    const rec = record({
      status: 'completed',
      counters: { ...record().counters, diagnostics: 1 },
      routing: { investigate: true, reasons: ['diagnostics', 'low_confidence'] },
    })
    expect(routingSentence(rec, triage('needs_investigation', 0.71), LIMITS)).toBe(
      'Sent to investigation: a rule found a signal, and confidence 0.71 is below 0.80.',
    )
    expect(
      routingSentence({ ...rec, routing: { investigate: true, reasons: ['manual_request'] } }, undefined, LIMITS),
    ).toBe('Sent to investigation: it was requested manually.')
    expect(
      routingSentence(
        { ...rec, routing: { investigate: true, reasons: ['diagnostics', 'needs_investigation', 'low_confidence'] } },
        triage('needs_investigation', 0.7),
        LIMITS,
      ),
    ).toBe(
      'Sent to investigation: a rule found a signal, triage chose needs_investigation, and confidence 0.70 is below 0.80.',
    )
    expect(
      routingSentence(
        { ...rec, routing: { investigate: false, reasons: [] } },
        triage('expected_behavior', 0.92),
        LIMITS,
      ),
    ).toBe(
      'Not sent to investigation: no rule found a signal, and triage chose expected_behavior with confidence 0.92.',
    )
    expect(routingSentence({ ...rec, routing: undefined }, undefined, LIMITS)).toBeUndefined()
  })

  it('orders the triage bars and flags the chosen one', () => {
    const answer = triage('needs_investigation', 0.71).answers.investigation
    if (answer.type !== 'choice') throw new Error('choice expected')
    expect(choiceBars(answer).map((bar) => [bar.key, bar.probability, bar.chosen])).toEqual([
      ['needs_investigation', 0.71, true],
      ['insufficient_evidence', 0.21, false],
      ['expected_behavior', 0.08, false],
    ])
  })

  it('finds where a cited entry can be read', () => {
    const snapshot = {
      sessions: [{ session_id: 's_root', preview: [{ entry_id: 'e_t_a_fc_1' }, 'text'] }],
      diagnostics: [
        {
          rule_id: 'r',
          fingerprint: 'f',
          session_id: 's_root',
          evidence: [{ session_id: 's_root', entry_id: 'e_t_a_fc_9' }],
        },
      ],
    } as unknown as Snapshot
    expect(locateEntry(snapshot, { session_id: 's_root', entry_id: 'e_t_a_fc_1' })).toBe('preview')
    expect(locateEntry(snapshot, { session_id: 's_root', entry_id: 'e_t_a_fc_9' })).toBe('signal')
    expect(locateEntry(snapshot, { session_id: 's_root', entry_id: 'nope' })).toBeUndefined()
    expect(locateEntry(undefined, { session_id: 's_root', entry_id: 'x' })).toBeUndefined()
    expect(signalFooter(snapshot.diagnostics[0], '7c1d…e2')).toBe('fp 7c1d…e2 · s_root · fc_9')
  })

  it('writes the meta line after the session id and the probe footnote', () => {
    const snapshot = { captured_at: T0 + 7000, coverage: { sessions_in_scope: 3 } } as unknown as Snapshot
    expect(metaRest(record(), snapshot)).toBe(`turn t_01234567… · root + 2 descendants · captured ${clock(T0 + 7000)}`)
    expect(metaRest(record(), undefined)).toBe('turn t_01234567…')
    expect(probeParts({ session_id: 's', target: 'engine::triggers::info', code: 'NOT_FOUND', calls: [] })).toEqual({
      call: 'engine::triggers::info → NOT_FOUND',
      note: 'in the default namespace',
    })
    expect(probeParts({ session_id: 's', target: 'x::y', code: 'E', calls: [] }).note).toBeUndefined()
  })
})

describe('code references', () => {
  const range = { path: 'harness/src/registry_notice.rs', line_from: 120, line_to: 148 }
  const one = { path: 'state/src/kv.rs', line_from: 88, line_to: 88 }

  it('shows an en dash and copies an ASCII hyphen', () => {
    expect(codeRefLabel(range)).toBe('harness/src/registry_notice.rs:120\u2013148')
    expect(codeRefCopy(range)).toBe('harness/src/registry_notice.rs:120-148')
  })

  it('says a single line once', () => {
    expect(codeRefLabel(one)).toBe('state/src/kv.rs:88')
    expect(codeRefCopy(one)).toBe('state/src/kv.rs:88')
  })

  it('reads aloud as lines, not as a path with colons', () => {
    expect(codeRefAria(range)).toBe('Copy harness/src/registry_notice.rs lines 120 to 148')
    expect(codeRefAria(one)).toBe('Copy state/src/kv.rs line 88')
  })

  it('does not touch a hyphen inside a file name', () => {
    expect(codeRefCopy({ path: 'context-manager/src/prune-old.rs', line_from: 1, line_to: 2 })).toBe(
      'context-manager/src/prune-old.rs:1-2',
    )
  })
})

describe('pathSegments', () => {
  it('breaks after each slash, never inside a name', () => {
    expect(pathSegments('/home/layon/workspaces/workers')).toEqual(['/home/', 'layon/', 'workspaces/', 'workers'])
    expect(pathSegments('/srv/workers/')).toEqual(['/srv/', 'workers/'])
    expect(pathSegments('workers')).toEqual(['workers'])
  })

  it('always joins back to the path', () => {
    for (const path of ['/a//b', '/', '', '/x y/z']) expect(pathSegments(path).join('')).toBe(path)
  })
})

describe('investigationCaps', () => {
  it('takes the code-access caps only when the analysis has a directory', () => {
    expect(investigationCaps(LIMITS, false)).toEqual({ steps: 1, totalTokens: 200_000 })
    expect(investigationCaps(LIMITS, true)).toEqual({ steps: 32, totalTokens: 800_000 })
  })
})
