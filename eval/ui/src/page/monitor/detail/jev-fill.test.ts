import { describe, expect, it } from 'vitest'
import type { ProposeValidationResponse } from '../../../types'
import {
  choosePair,
  classifyFillFailure,
  editedNotice,
  excludedSummary,
  fillFromResponse,
  needsNewCase,
  noFitNotice,
  noPairNotice,
  noRunsNotice,
  offeredLine,
  type Proposed,
  pct,
  planHint,
  proposedNotice,
  quickPicks,
  settlePicked,
  stackLine,
} from './jev-fill'

const response = (patch: Partial<ProposeValidationResponse> = {}): ProposeValidationResponse => ({
  outcome: 'proposed',
  proposal: {
    baseline_execution_id: 'base',
    candidate_execution_id: 'cand',
    confidence: 0.98,
    low_confidence: false,
    stack_note: 'recorded stack differs: harness 1.8.39·cc6b778 → 1.8.39·ab17d27',
  },
  runs_listed: 20,
  runs_considered: 4,
  pairs_considered: 6,
  pairs_dropped: 0,
  excluded: { running: 2, technical_failed: 1 },
  alternatives: [],
  ...patch,
})

const withAlternatives = (low = false): Proposed =>
  fillFromResponse(
    response({
      proposal: { ...response().proposal!, confidence: 0.43, low_confidence: low },
      alternatives: [
        {
          baseline_execution_id: 'a1',
          candidate_execution_id: 'a2',
          probability: 0.21,
          stack_note: 'recorded stacks identical',
        },
        { baseline_execution_id: 'b1', candidate_execution_id: 'b2', probability: 0.06999999, stack_note: '' },
      ],
    }),
  ) as Proposed

describe('pct', () => {
  it('rounds to a whole percent', () => {
    expect(pct(0.86)).toBe('86%')
    expect(pct(0.06999999)).toBe('7%')
    expect(pct(0.9849)).toBe('98%')
    expect(pct(1)).toBe('100%')
    expect(pct(0)).toBe('0%')
    expect(pct(0.002)).toBe('<1%')
  })
})

describe('fillFromResponse', () => {
  it('turns a proposal into Jev pair first, alternatives after, both pickers marked', () => {
    const fill = withAlternatives(true)
    expect(fill.pairs.map((pair) => [pair.baseline, pair.candidate, pair.probability])).toEqual([
      ['base', 'cand', 0.43],
      ['a1', 'a2', 0.21],
      ['b1', 'b2', 0.06999999],
    ])
    expect(fill).toMatchObject({ chosen: 0, lowConfidence: true, marks: { baseline: true, candidate: true } })
  })

  it('keeps the other outcomes apart and refuses a proposed answer without a pair', () => {
    expect(fillFromResponse(response({ outcome: 'none_fits', proposal: undefined })).kind).toBe('none_fits')
    expect(fillFromResponse(response({ outcome: 'no_comparable_pair', proposal: undefined })).kind).toBe(
      'no_comparable_pair',
    )
    expect(fillFromResponse(response({ proposal: undefined })).kind).toBe('failed')
  })
})

describe('the notice', () => {
  it('says how sure Jev is, in whole percent', () => {
    const notice = proposedNotice(fillFromResponse(response()) as Proposed, 'tool_contract_recovery')
    expect(notice).toMatchObject({ tone: 'neutral', headline: 'Jev picked this pair · 98% confidence', advice: [] })
    expect(notice.foot).toBe(
      'Chosen among 6 comparable pairs from 4 runs; 3 runs excluded: 2 still running, 1 technical failure.',
    )
    expect(notice.stack).toEqual({
      head: 'Recorded stack differs:',
      mono: 'harness 1.8.39·cc6b778 → 1.8.39·ab17d27',
    })
  })

  it('turns warn on the backend low-confidence flag, not on a percentage', () => {
    expect(proposedNotice(withAlternatives(true), 'tool_contract_recovery')).toMatchObject({
      tone: 'warn',
      headline: "Jev isn't sure · 43% confidence",
    })
    expect(proposedNotice(withAlternatives(false), 'tool_contract_recovery').tone).toBe('neutral')
  })

  it('names an alternative by its own percentage and keeps the original tone', () => {
    const alternative = choosePair(withAlternatives(true), 1)
    expect(proposedNotice(alternative, 'tool_contract_recovery')).toMatchObject({
      tone: 'warn',
      headline: "Jev's alternative · 21%",
      stack: { head: 'Recorded stacks identical' },
    })
  })
})

describe('what a low-confidence answer asks beyond checking', () => {
  it('points at the other pairs, and at E2E when the plan needs a new case', () => {
    const low = withAlternatives(true)
    expect(proposedNotice(low, 'tool_contract_recovery').advice).toEqual(['Compare with the other pairs below.'])
    expect(proposedNotice(low, null).advice).toEqual([
      'Compare with the other pairs below; if none tests this change, run the new case in E2E first.',
    ])
    const alone = fillFromResponse(
      response({ proposal: { ...response().proposal!, confidence: 0.4, low_confidence: true } }),
    ) as Proposed
    expect(proposedNotice(alone, null).advice).toEqual([
      'If no recorded pair tests this change, run the new case in E2E first.',
    ])
  })

  it('stays quiet when Jev is sure, however the plan reads', () => {
    expect(proposedNotice(withAlternatives(false), null).advice).toEqual([])
  })

  it('promotes E2E only for Jev low-confidence picks on a plan without a scenario', () => {
    const low = withAlternatives(true)
    expect(needsNewCase(low, null)).toBe(true)
    expect(needsNewCase(low, 'tool_contract_recovery')).toBe(false)
    expect(needsNewCase(withAlternatives(false), null)).toBe(false)
    expect(
      needsNewCase(
        settlePicked(settlePicked(low, { baseline: 'x', candidate: 'cand' }), { baseline: 'x', candidate: 'y' }),
        null,
      ),
    ).toBe(false)
    expect(needsNewCase({ kind: 'idle' }, null)).toBe(false)
  })
})

describe('excluded runs', () => {
  it('humanizes and pluralizes, statuses by count then the scenario and id counts', () => {
    expect(
      excludedSummary({
        other_scenario: 43,
        cancelled: 5,
        infra_failed: 2,
        incomplete: 10,
        technical_failed: 17,
        no_id: 1,
      }),
    ).toBe(
      '17 technical failures, 10 incomplete, 5 cancelled, 2 infra failures, 43 on other scenarios, 1 without an id',
    )
    expect(excludedSummary({ other_scenario: 1, running: 1, technical_failed: 1, unknown_status: 1 })).toBe(
      '1 still running, 1 technical failure, 1 with no status, 1 on another scenario',
    )
    expect(excludedSummary({})).toBe('')
  })

  it('adds the older pairs that were not offered, and drops an empty exclusion clause', () => {
    const tally = { runsConsidered: 12, pairsConsidered: 60, pairsDropped: 72, excluded: { running: 2 } }
    expect(offeredLine('Chosen among', tally)).toBe(
      'Chosen among 60 comparable pairs from 12 runs; 2 runs excluded: 2 still running; 72 older pairs not offered.',
    )
    expect(
      offeredLine('Looked at', { ...tally, pairsConsidered: 1, runsConsidered: 1, pairsDropped: 1, excluded: {} }),
    ).toBe('Looked at 1 comparable pair from 1 run; 1 older pair not offered.')
  })
})

describe('editing after a fill', () => {
  const picked = (fill: Proposed, baseline: string, candidate: string) =>
    settlePicked(fill, { baseline, candidate }) as Proposed

  it('drops only the mark of the field that no longer holds what Jev put there', () => {
    const fill = withAlternatives()
    expect(editedNotice(fill)).toBeUndefined()
    const candidate = picked(fill, 'base', 'other')
    expect(candidate.marks).toEqual({ baseline: true, candidate: false })
    expect(editedNotice(candidate)).toEqual({
      headline: 'You changed the candidate Jev picked',
      detail: "Jev's 43% confidence was for the pair as Jev chose it, so it no longer applies.",
    })
    expect(editedNotice(picked(fill, 'other', 'cand'))?.headline).toBe('You changed the baseline Jev picked')
    expect(editedNotice(picked(candidate, 'other', 'other'))?.headline).toBe('You changed both runs Jev picked')
  })

  it('follows the ids, not the clicks: putting Jev back, or one of its alternatives, is not a change', () => {
    const fill = withAlternatives()
    const changed = picked(fill, 'base', 'other')
    // Re-picking Jev's own candidate restores the mark.
    expect(picked(changed, 'base', 'cand')).toMatchObject({ chosen: 0, marks: { baseline: true, candidate: true } })
    expect(editedNotice(picked(changed, 'base', 'cand'))).toBeUndefined()
    // Typing in the pair of an alternative is that alternative, with its own percentage.
    const alternative = picked(fill, 'a1', 'a2')
    expect(alternative).toMatchObject({ chosen: 1, marks: { baseline: true, candidate: true } })
    expect(proposedNotice(alternative, 's').headline).toBe("Jev's alternative · 21%")
    // Half an alternative is still a change of the pair that was on the pickers.
    expect(picked(alternative, 'a1', 'cand').marks).toEqual({ baseline: true, candidate: false })
  })

  it('leaves the other states alone', () => {
    expect(settlePicked({ kind: 'idle' }, { baseline: 'x', candidate: 'y' })).toEqual({ kind: 'idle' })
  })
})

describe('quick picks', () => {
  const picked = (fill: Proposed, baseline: string, candidate: string) =>
    settlePicked(fill, { baseline, candidate }) as Proposed

  it('offers every other pair, Jev pick included once an alternative is on the pickers', () => {
    const fill = withAlternatives()
    expect(quickPicks(fill).map((pick) => [pick.index, pick.quiet])).toEqual([
      [1, '21% · recorded stacks identical'],
      [2, '7%'],
    ])
    expect(quickPicks(choosePair(fill, 1)).map((pick) => [pick.index, pick.quiet])).toEqual([
      [0, "Jev's pick · 43% · recorded stack differs: harness 1.8.39·cc6b778 → 1.8.39·ab17d27"],
      [2, '7%'],
    ])
  })

  it('stays while one picker is Jev, lists every pair then, and goes when both were changed', () => {
    const one = picked(withAlternatives(), 'base', 'other')
    expect(quickPicks(one).map((pick) => pick.index)).toEqual([0, 1, 2])
    expect(quickPicks(picked(one, 'other', 'other'))).toEqual([])
    expect(quickPicks(choosePair(picked(one, 'other', 'other'), 1)).map((pick) => pick.index)).toEqual([0, 2])
  })

  it('has nothing to offer without alternatives, until a picker was edited: then Jev pick is the way back', () => {
    const alone = fillFromResponse(response()) as Proposed
    expect(quickPicks(alone)).toEqual([])
    expect(quickPicks(picked(alone, 'base', 'other')).map((pick) => [pick.index, pick.quiet])).toEqual([
      [0, "Jev's pick · 98% · recorded stack differs: harness 1.8.39·cc6b778 → 1.8.39·ab17d27"],
    ])
  })
})

describe('no pair', () => {
  const tally = {
    runsConsidered: 3,
    pairsConsidered: 0,
    pairsDropped: 0,
    excluded: { running: 1, technical_failed: 1 },
  }

  it('says why code found no pair, by the rule that failed', () => {
    expect(noPairNotice(tally, 'tool_contract_recovery')).toEqual({
      headline: 'No comparable pair yet',
      detail: '3 finished runs include tool_contract_recovery, but no two used the same model and provider.',
      foot: 'Not counted: 1 still running, 1 technical failure. Run the scenario again in E2E with the changed Harness, then come back.',
    })
    expect(noPairNotice({ ...tally, runsConsidered: 4, excluded: {} }, null)).toMatchObject({
      detail: '4 finished runs, but no two ran the same scenarios with the same model.',
      foot: 'Run the case again in E2E with the changed Harness, then come back.',
    })
    expect(noPairNotice({ ...tally, runsConsidered: 0 }, 'x').detail).toBe('No finished run includes x.')
  })

  it('says what Jev looked at when it found no fit', () => {
    expect(noFitNotice({ ...tally, pairsConsidered: 6, runsConsidered: 4 })).toMatchObject({
      headline: 'Jev found no pair for this change',
      detail:
        "None of the 6 comparable pairs looks like this suggestion's baseline and candidate. Choose the runs yourself, or run the pair in E2E first.",
      foot: 'Looked at 6 comparable pairs from 4 runs; 2 runs excluded: 1 still running, 1 technical failure.',
    })
  })
})

describe('copy', () => {
  it('builds the plan hint from the scenario', () => {
    expect(planHint('tool_contract_recovery')).toMatch(/^Run `tool_contract_recovery` twice with the same model/)
    expect(planHint(null)).toMatch(/^This plan needs a new E2E case/)
    expect(noRunsNotice('s1').detail).toContain('Run s1 twice there')
    expect(noRunsNotice(null).detail).toContain('Run the case twice there')
  })

  it('splits the stack note into words and values', () => {
    expect(stackLine('recorded stacks identical')).toEqual({ head: 'Recorded stacks identical' })
    expect(stackLine('')).toEqual({ head: '' })
  })
})

describe('classifyFillFailure', () => {
  it('shows the provider text as returned, without the code', () => {
    expect(
      classifyFillFailure(
        'dependency error: jev_unavailable: Jev (typesafe) answered provider_error (HTTP 402): no available TypeSafe API credits',
      ),
    ).toEqual({
      kind: 'jev',
      message: 'Jev (typesafe) answered provider_error (HTTP 402): no available TypeSafe API credits',
    })
    expect(classifyFillFailure('jev_invalid_response: no choice')).toEqual({ kind: 'jev', message: 'no choice' })
    expect(classifyFillFailure('dependency error: jev_invalid_request: the request is malformed (HTTP 400)')).toEqual({
      kind: 'jev',
      message: 'the request is malformed (HTTP 400)',
    })
  })

  it('reads a down E2E service apart, and keeps any other message whole', () => {
    expect(classifyFillFailure('e2e_unavailable: the E2E service could not list executions: timeout')).toEqual({
      kind: 'e2e_unavailable',
    })
    expect(classifyFillFailure('trigger timed out after 90000 ms')).toEqual({
      kind: 'jev',
      message: 'trigger timed out after 90000 ms',
    })
  })
})
