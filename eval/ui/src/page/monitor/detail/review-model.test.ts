import { describe, expect, it } from 'vitest'
import type { Criterion, EvidenceRun, EvidenceSide, SuggestionReview, ValidationLink } from '../../../types'
import {
  criterionFrozen,
  criterionSentence,
  criterionSummary,
  defaultDraft,
  disagrees,
  draftOf,
  duplicateLabel,
  effectUnit,
  firstDataAt,
  historyRows,
  improvementRefusal,
  lifecycleBadge,
  metricChoices,
  moveRefusal,
  parseCriterion,
  parseDuplicateOf,
  parsePr,
  parseVersion,
  prLabel,
  readPattern,
  registeredLate,
  sameVerdict,
  statusSentence,
  verdictControls,
  withChoice,
} from './review-model'

const PATTERN = 'repeated_contract_discovery:engine::functions::info'
const at = (hour: number, minute = 0) => new Date(2026, 9, 3, hour, minute).getTime()

const criterion = (over: Partial<Criterion> = {}): Criterion => ({
  metric: 'signal_per_run',
  pattern: PATTERN,
  direction: 'decrease',
  min_effect: 0.5,
  min_runs: 5,
  scenario_id: 'tool_contract_recovery',
  registered_at: at(13, 40),
  registered_by: 'layon',
  ...over,
})

const runs = (count: number, over: Partial<EvidenceRun> = {}): EvidenceRun[] =>
  Array.from({ length: count }, (_, i) => ({
    run_id: `r${i}`,
    completed: true,
    completion: 'completed',
    technical: 'valid',
    ...over,
  }))

const side = (list: EvidenceRun[]): EvidenceSide => ({
  execution_id: 'plan-x',
  runs: list,
  n: list.filter((r) => r.completed).length,
  mean: 1,
})

function review(over: Partial<SuggestionReview> = {}): SuggestionReview {
  return {
    evaluation_id: 'eval_1',
    suggestion_index: 0,
    title: 't',
    harness_component: 'c',
    patterns: [PATTERN],
    lifecycle: { status: 'new', history: [] },
    ...over,
  }
}

const link = (over: Partial<ValidationLink> = {}): ValidationLink => ({
  suggestion_index: 0,
  baseline: {} as ValidationLink['baseline'],
  candidate: {} as ValidationLink['candidate'],
  comparability: { comparable: true, checks: [] },
  attached_at: at(13, 52),
  ...over,
})

/** A review that passes every rule: criterion first, a pair after it, five runs a side. */
const ready = (): { row: SuggestionReview; links: ValidationLink[] } => ({
  row: review({
    criterion: criterion(),
    evidence: {
      scenario_id: 'tool_contract_recovery',
      computed_at: at(13, 53),
      baseline: side(runs(5)),
      candidate: side(runs(5)),
      computed_outcome: 'validated_improvement',
      reason: 'ok',
    },
  }),
  links: [link()],
})

describe('lifecycle', () => {
  it('words each state, with the pull request and version when there are some', () => {
    expect(lifecycleBadge({ status: 'new', history: [] })).toEqual({ label: 'New', tone: 'neutral' })
    expect(lifecycleBadge({ status: 'accepted', history: [] }).strong).toBe(true)
    expect(lifecycleBadge({ status: 'in_progress', history: [] }).tone).toBe('warn')
    expect(lifecycleBadge({ status: 'shipped', pr: '#1292', history: [] })).toEqual({
      label: 'Shipped in PR #1292',
      tone: 'ok',
    })
    expect(
      lifecycleBadge({
        status: 'shipped',
        pr: 'https://github.com/iii-hq/workers/pull/1292',
        version: '1.8.44',
        history: [],
      }).label,
    ).toBe('Shipped in PR #1292 · v1.8.44')
    expect(lifecycleBadge({ status: 'rejected', reason: 'Not reproducible', history: [] }).label).toBe(
      'Rejected · not reproducible',
    )
    expect(lifecycleBadge({ status: 'duplicate', duplicate_of: '#1292', history: [] }).label).toBe(
      'Duplicate of PR #1292',
    )
    expect(lifecycleBadge({ status: 'duplicate', duplicate_of: 'eval_e256d7fc1a:1', history: [] }).label).toBe(
      'Duplicate of eval_e256d… · S2',
    )
  })

  it('offers the moves eval::review accepts', () => {
    expect(moveRefusal('new', 'accepted')).toBeUndefined()
    expect(moveRefusal('rejected', 'in_progress')).toBeUndefined()
    expect(moveRefusal('accepted', 'new')).toMatch(/go back to New/)
    expect(moveRefusal('shipped', 'rejected')).toMatch(/stays shipped/)
    // Completing a shipped suggestion's pull request or version is shipped again.
    expect(moveRefusal('shipped', 'shipped')).toBeUndefined()
  })

  it('reads a pull request in the forms people paste', () => {
    expect(parsePr('1292')).toBe('#1292')
    expect(parsePr(' #1292 ')).toBe('#1292')
    expect(parsePr('iii-hq/workers#1292')).toBe('iii-hq/workers#1292')
    expect(parsePr('https://github.com/iii-hq/workers/pull/1292')).toBe('https://github.com/iii-hq/workers/pull/1292')
    expect(parsePr('soon')).toBeUndefined()
    expect(prLabel('iii-hq/workers#1292')).toBe('iii-hq/workers#1292')
    expect(duplicateLabel('#7')).toBe('PR #7')
  })

  it('reads a duplicate as a pull request or another suggestion, never itself', () => {
    const own = { evaluationId: 'eval_1', index: 0 }
    expect(parseDuplicateOf('#12', own)).toBe('#12')
    expect(parseDuplicateOf('S2', own)).toBe('eval_1:1')
    expect(parseDuplicateOf('eval_9f · S3', own)).toBe('eval_9f:2')
    expect(parseDuplicateOf('eval_9f:S1', own)).toBe('eval_9f:0')
    expect(parseDuplicateOf('S1', own)).toBeUndefined()
    expect(parseDuplicateOf('S0', own)).toBeUndefined()
    expect(parseDuplicateOf('the other one', own)).toBeUndefined()
  })

  it('takes a semantic version, with or without the v', () => {
    expect(parseVersion('v1.8.44')).toBe('1.8.44')
    expect(parseVersion('1.8.44-rc.1')).toBe('1.8.44-rc.1')
    expect(parseVersion('1.8')).toBeUndefined()
    expect(parseVersion('latest')).toBeUndefined()
  })

  it('keeps who changed what and when, newest first', () => {
    const lifecycle = {
      status: 'in_progress' as const,
      history: [
        { status: 'accepted' as const, at: at(9), by: 'layon' },
        { status: 'in_progress' as const, at: at(10, 12), by: 'ana', note: 'on it' },
      ],
    }
    expect(historyRows(lifecycle).map((row) => [row.label, row.by, row.at, row.note])).toEqual([
      ['In progress', 'ana', '03 Oct 10:12', 'on it'],
      ['Accepted', 'layon', '03 Oct 09:00', undefined],
    ])
    expect(statusSentence(lifecycle)).toBe('In progress by ana, 03 Oct 10:12')
    expect(statusSentence({ status: 'new', history: [] })).toBeUndefined()
  })
})

describe('criterion form', () => {
  it('starts from the signal the suggestion cites, else function calls', () => {
    expect(defaultDraft([PATTERN], 'tool_contract_recovery')).toEqual({
      choice: `signal_per_run:${PATTERN}`,
      direction: 'decrease',
      effect: '1',
      runs: '5',
      scenario: 'tool_contract_recovery',
    })
    expect(defaultDraft([], undefined).choice).toBe('function_calls')
    expect(metricChoices([PATTERN]).map((choice) => choice.value)).toEqual([
      `signal_per_run:${PATTERN}`,
      'function_calls',
      'tokens',
      'cost_usd',
      'duration',
      'pass_rate',
    ])
  })

  it('turns the draft into what eval::review takes: the number typed, in the metric’s own unit', () => {
    // One signal per run is 1, not a fraction of the baseline.
    expect(parseCriterion(defaultDraft([PATTERN], 's'))).toEqual({
      ok: true,
      scenarioId: 's',
      input: { metric: 'signal_per_run', pattern: PATTERN, direction: 'decrease', min_effect: 1, min_runs: 5 },
    })
    expect(parseCriterion({ ...defaultDraft([PATTERN], 's'), effect: '0,25' })).toMatchObject({
      ok: true,
      input: { min_effect: 0.25 },
    })
    expect(parseCriterion({ ...defaultDraft([], 's'), effect: '12,5', direction: 'increase' })).toMatchObject({
      ok: true,
      input: { metric: 'function_calls', direction: 'increase', min_effect: 12.5 },
    })
    expect(parseCriterion({ ...defaultDraft([], 's'), choice: 'cost_usd', effect: '0.01' })).toMatchObject({
      ok: true,
      input: { metric: 'cost_usd', min_effect: 0.01 },
    })
  })

  it('names the unit of each metric and keeps an untouched starting value from changing unit silently', () => {
    expect(
      ['signal_per_run:x:y', 'pass_rate', 'cost_usd', 'duration', 'tokens', 'function_calls'].map(effectUnit),
    ).toEqual(['signals per run', 'percentage points', 'USD', 'seconds', 'tokens', 'function calls'])
    const draft = defaultDraft([PATTERN], 's')
    expect(withChoice(draft, 'cost_usd')).toMatchObject({ choice: 'cost_usd', effect: '0.01' })
    expect(withChoice(draft, 'pass_rate').effect).toBe('10')
    // A value the person typed stays.
    expect(withChoice({ ...draft, effect: '3' }, 'cost_usd').effect).toBe('3')
  })

  it('says what is wrong under each field', () => {
    const base = defaultDraft([PATTERN], 's')
    expect(parseCriterion({ ...base, runs: '2' })).toEqual({
      ok: false,
      errors: { runs: 'At least 3 runs per side. The E2E advises 5.' },
    })
    expect(parseCriterion({ ...base, runs: '21' })).toMatchObject({
      ok: false,
      errors: { runs: 'At most 20 runs per side.' },
    })
    expect(parseCriterion({ ...base, effect: '0' })).toMatchObject({
      ok: false,
      errors: { effect: 'Enter a number above 0, in signals per run.' },
    })
    expect(parseCriterion({ ...base, effect: 'abc' }).ok).toBe(false)
    // In the metric's own unit, 120 signals or 120 seconds is a number like any other.
    expect(parseCriterion({ ...base, effect: '120' }).ok).toBe(true)
    expect(parseCriterion({ ...base, choice: 'duration', effect: '120', direction: 'increase' }).ok).toBe(true)
    // A pass rate cannot move by more than all of it.
    expect(parseCriterion({ ...base, choice: 'pass_rate', effect: '100' }).ok).toBe(true)
    expect(parseCriterion({ ...base, choice: 'pass_rate', effect: '120' })).toMatchObject({
      ok: false,
      errors: { effect: expect.stringContaining('100 percentage points') },
    })
    expect(parseCriterion({ ...base, scenario: ' ' })).toMatchObject({
      ok: false,
      errors: { scenario: expect.any(String) },
    })
  })

  it('summarises a criterion and reads it back into a draft', () => {
    expect(criterionSummary(criterion({ min_effect: 1 }))).toBe(
      'repeated_contract_discovery per run · lower is better · at least 1.0 fewer signals per run · 5 runs per side',
    )
    const summary = (over: Partial<Criterion>) =>
      criterionSummary(criterion({ pattern: undefined, ...over })).split(' · ')[2]
    expect(summary({ metric: 'pass_rate', direction: 'increase', min_effect: 5 })).toBe(
      'at least 5 pp higher pass rate',
    )
    expect(summary({ metric: 'pass_rate', direction: 'decrease', min_effect: 12.5 })).toBe(
      'at least 12.5 pp lower pass rate',
    )
    expect(summary({ metric: 'cost_usd', min_effect: 0.01 })).toBe('at least $0.01 cheaper per run')
    expect(summary({ metric: 'cost_usd', direction: 'increase', min_effect: 0.25 })).toBe(
      'at least $0.25 more expensive per run',
    )
    expect(summary({ metric: 'duration', min_effect: 5 })).toBe('at least 5 s faster per run')
    expect(summary({ metric: 'duration', direction: 'increase', min_effect: 1.5 })).toBe(
      'at least 1.5 s slower per run',
    )
    expect(summary({ metric: 'tokens', min_effect: 1000 })).toBe('at least 1,000 fewer tokens per run')
    expect(summary({ metric: 'function_calls', direction: 'increase', min_effect: 2 })).toBe(
      'at least 2 more function calls per run',
    )
    expect(summary({ metric: 'signal_per_run', pattern: PATTERN, min_effect: 0.25 })).toBe(
      'at least 0.25 fewer signals per run',
    )
    // What is shown is the threshold the backend compares against, never a rounded one.
    expect(summary({ metric: 'cost_usd', min_effect: 0.015 })).toBe('at least $0.015 cheaper per run')
    expect(summary({ metric: 'signal_per_run', pattern: PATTERN, min_effect: 0.125 })).toBe(
      'at least 0.125 fewer signals per run',
    )
    expect(summary({ metric: 'pass_rate', direction: 'increase', min_effect: 2.55 })).toBe(
      'at least 2.55 pp higher pass rate',
    )
    expect(criterionSentence(criterion())).toContain('(registered 13:40 by layon)')
    expect(criterionSentence(undefined)).toBeUndefined()
    expect(draftOf(criterion({ min_effect: 0.25 }))).toMatchObject({
      choice: `signal_per_run:${PATTERN}`,
      effect: '0.25',
      runs: '5',
    })
  })
})

describe('when the criterion was registered', () => {
  it('counts an attached pair, or an execution the worker started, as the first results', () => {
    expect(firstDataAt(review(), [])).toBeUndefined()
    expect(firstDataAt(review(), [link({ attached_at: at(14) }), link({ attached_at: at(13) })])).toBe(at(13))
    // Another suggestion's pair is not this one's.
    expect(firstDataAt(review(), [link({ suggestion_index: 1 })])).toBeUndefined()
    const started = review({
      run: {
        baseline_execution_id: 'plan-a',
        baseline_commit: 'a',
        candidate_commit: 'b',
        scenario_id: 's',
        runs: 5,
        model: 'm',
        provider: 'p',
        started_at: at(12),
        state: 'running',
      },
    })
    expect(firstDataAt(started, [link()])).toBe(at(12))
    // Nothing was accepted by the E2E yet: no results.
    expect(
      firstDataAt(review({ run: { ...started.run!, baseline_execution_id: undefined, state: 'starting' } }), []),
    ).toBeUndefined()
  })

  it('counts an attached execution from its own start, and a replaced run from the first one', () => {
    const old = link({
      attached_at: at(15),
      baseline: { started_at: at(11) } as ValidationLink['baseline'],
      candidate: { started_at: at(10, 30) } as ValidationLink['candidate'],
    })
    // The pair was attached at 15:00, its executions ran from 10:30.
    expect(firstDataAt(review(), [old])).toBe(at(10, 30))
    // A criterion written at 14:00 is late for it, however recent the attach.
    expect(registeredLate(review({ criterion: criterion({ registered_at: at(14) }) }), [old])).toBe(at(10, 30))
    // A restart replaced the run, not what was seen of the one before.
    expect(firstDataAt(review({ first_run_at: at(9) }), [])).toBe(at(9))
    expect(firstDataAt(review({ first_run_at: at(9) }), [old])).toBe(at(9))
  })

  it('freezes the criterion once results exist, and flags one registered after them', () => {
    const early = review({ criterion: criterion() })
    expect(criterionFrozen(early, [])).toBe(false)
    expect(criterionFrozen(early, [link()])).toBe(true)
    expect(criterionFrozen(review(), [link()])).toBe(false)
    expect(registeredLate(early, [link()])).toBeUndefined()
    expect(
      registeredLate(review({ criterion: criterion({ registered_at: at(15, 2) }) }), [
        link({ attached_at: at(13, 41) }),
      ]),
    ).toBe(at(13, 41))
  })
})

describe('refusing validated improvement', () => {
  it('allows it when the criterion came first and both sides have the runs', () => {
    const { row, links } = ready()
    expect(improvementRefusal(row, links)).toBeUndefined()
  })

  it('says why not, case by case', () => {
    const { row, links } = ready()
    expect(improvementRefusal({ ...row, criterion: undefined }, links)).toBe(
      'Validated improvement needs a criterion. Register it, then run again.',
    )
    expect(improvementRefusal(row, [])).toMatch(/no E2E runs are attached or running/)
    expect(
      improvementRefusal({ ...row, criterion: criterion({ registered_at: at(15, 2) }) }, [
        link({ attached_at: at(13, 41) }),
      ]),
    ).toBe(
      "Not available: the criterion was registered at 15:02, after the first run (13:41). A criterion chosen after seeing results can't validate an improvement.",
    )
    expect(improvementRefusal({ ...row, evidence: undefined }, links)).toMatch(/no evidence was computed/)
    expect(improvementRefusal({ ...row, criterion: criterion({ scenario_id: 'other' }) }, links)).toMatch(
      /evidence is of tool_contract_recovery, the criterion measures other/,
    )
  })

  it('names the run that did not finish, or counts the runs when all did', () => {
    const { row, links } = ready()
    const evidence = row.evidence!
    const interrupted = [
      ...runs(3),
      { run_id: 'r3', completed: false, completion: 'interrupted', technical: 'valid' },
      ...runs(1),
    ]
    expect(improvementRefusal({ ...row, evidence: { ...evidence, candidate: side(interrupted) } }, links)).toBe(
      "Not available: the candidate's run 4 was interrupted. 4 of 5 runs finished.",
    )
    const broken = [
      { run_id: 'r0', completed: false, completion: 'completed', technical: 'technical_failed' },
      ...runs(4),
    ]
    expect(improvementRefusal({ ...row, evidence: { ...evidence, baseline: side(broken) } }, links)).toMatch(
      /baseline's run 1 had an infrastructure failure \(technical failed\)\. 4 of 5/,
    )
    expect(improvementRefusal({ ...row, evidence: { ...evidence, baseline: side(runs(3)) } }, links)).toBe(
      "Not available: 3 counted runs on the baseline, below the criterion's minimum of 5.",
    )
  })

  it('refuses runs that are not comparable, naming what differs', () => {
    const { row } = ready()
    const different = link({
      comparability: {
        comparable: false,
        checks: [
          { field: 'model', matches: false },
          { field: 'provider', matches: false },
          { field: 'scenarios', matches: true },
        ],
      },
    })
    expect(improvementRefusal(row, [different])).toBe(
      "Not available: the runs aren't comparable (Model, Provider differ).",
    )
  })
})

describe('the verdict', () => {
  it("offers the plan's controls and the three every validation needs, once each", () => {
    const controls = verdictControls({
      scenario_id: null,
      reproduction: 'r',
      invariants: ['schedule once', 'The candidate exercised the changed path'],
      primary_metric: 'm',
      expectation: 'e',
      non_regression_controls: ['recovery still happens'],
    })
    expect(controls).toEqual([
      'schedule once',
      'The candidate exercised the changed path',
      'recovery still happens',
      'Baseline and candidate differ only in the Harness build',
      'No infrastructure failures in the runs I counted',
    ])
  })

  it('notes when the person recorded something other than what the code computed', () => {
    const { row } = ready()
    const verdict = {
      outcome: 'no_improvement' as const,
      rationale: 'r',
      controls_checked: [],
      by: 'layon',
      at: at(10),
    }
    expect(disagrees(row)).toBe(false)
    expect(disagrees({ ...row, verdict })).toBe(true)
    expect(disagrees({ ...row, verdict: { ...verdict, outcome: 'validated_improvement' } })).toBe(false)
  })
})

describe('after release', () => {
  const pattern = (per?: number) => ({ pattern: PATTERN, occurrences: 0, analyses_with: 0, per_analysis: per })

  it('reads a pattern that fell by half or more as fell, otherwise as still there', () => {
    expect(readPattern(pattern(2.7), pattern(0.3))).toBe('fell')
    expect(readPattern(pattern(2), pattern(1))).toBe('fell')
    expect(readPattern(pattern(2.7), pattern(2.5))).toBe('still')
    expect(readPattern(pattern(2.7), undefined)).toBe('fell')
  })

  it('has nothing to fall from when the pattern was not there before', () => {
    expect(readPattern(pattern(0), pattern(1))).toBe('unseen')
    expect(readPattern(undefined, pattern(1))).toBe('unseen')
  })
})

describe('sameVerdict', () => {
  const current = {
    outcome: 'inconclusive' as const,
    rationale: 'n=1 per side',
    controls_checked: ['a', 'gone'],
    by: 'layon',
    at: 0,
  } as unknown as import('../../../types').Verdict
  const controls = ['a', 'b']
  it('is the verdict on file: same outcome, rationale and checked controls still in the plan', () => {
    expect(
      sameVerdict(current, { outcome: 'inconclusive', rationale: ' n=1 per side ', checked: ['a'] }, controls),
    ).toBe(true)
  })
  it('differs when anything a person can change differs', () => {
    expect(
      sameVerdict(current, { outcome: 'no_improvement', rationale: 'n=1 per side', checked: ['a'] }, controls),
    ).toBe(false)
    expect(sameVerdict(current, { outcome: 'inconclusive', rationale: 'more', checked: ['a'] }, controls)).toBe(false)
    expect(
      sameVerdict(current, { outcome: 'inconclusive', rationale: 'n=1 per side', checked: ['a', 'b'] }, controls),
    ).toBe(false)
    expect(sameVerdict(undefined, { outcome: 'inconclusive', rationale: 'x', checked: [] }, controls)).toBe(false)
  })
})
