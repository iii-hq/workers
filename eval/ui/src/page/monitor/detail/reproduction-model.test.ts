import { describe, expect, it } from 'vitest'
import type { Reply, Reproduction } from '../../../types'
import {
  compare,
  estimateCost,
  fisherTwoSided,
  moreSamplesNeeded,
  reproductionCost,
  reproductionSentence,
  standing,
  stepLabel,
  tally,
} from './reproduction-model'

const check = {
  decision_point: 'e_t_4c79_4_assistant',
  signal: { question: 'Does the reply use folder-relative globs?' },
  change: [],
}

function reply(index: number, signal: boolean | undefined, targets: string[], payload = '{}'): Reply {
  return {
    index,
    signal,
    calls: targets.map((target) => ({ target, payload })),
    thinking: '',
    text: '',
    usage: {},
    duration_ms: 1,
  }
}

function reproduction(
  kind: Reproduction['change_kind'],
  replies: Reply[],
  state: Reproduction['state'] = 'completed',
): Reproduction {
  return {
    id: `rep_${kind}`,
    change_kind: kind,
    change: [],
    check,
    model: 'm',
    requested: replies.length,
    state,
    samples: replies,
    original: reply(0, true, ['coder::search'], '{"path":"ade"}'),
    cost_unknown_samples: 0,
    judge_input_tokens: 0,
    judge_output_tokens: 0,
    by: 'ana',
    started_at: 1,
    updated_at: 1,
  }
}

/** The ADE experiment of 04/10: 3/20 fell into the trap; with the clearer contract 0/20, and `tree` 7 → 16. */
function adeBase(): Reproduction {
  return reproduction(
    'none',
    Array.from({ length: 20 }, (_, i) =>
      reply(i, i < 3, i < 3 ? ['coder::read-file', 'coder::search'] : i < 10 ? ['coder::read-file', 'coder::tree'] : ['coder::read-file'], i < 3 ? '{"path":"ade"}' : '{}'),
    ),
  )
}
function adeChange(): Reproduction {
  return reproduction(
    'custom',
    Array.from({ length: 20 }, (_, i) => reply(i, false, i < 16 ? ['coder::read-file', 'coder::tree'] : ['coder::read-file'])),
  )
}

describe('Fisher exact test', () => {
  it('matches the experiments', () => {
    // 3/20 vs 0/20: one-sided 0.115, two-sided twice that.
    expect(fisherTwoSided(3, 17, 0, 20)).toBeCloseTo(0.2308, 3)
    // tree 7/20 vs 16/20.
    expect(fisherTwoSided(7, 13, 16, 4)).toBeLessThan(0.01)
    expect(fisherTwoSided(5, 5, 5, 5)).toBeCloseTo(1, 6)
  })

  it('names how many more samples would settle a difference', () => {
    const more = moreSamplesNeeded({ positive: 3, measured: 20 }, { positive: 0, measured: 20 })
    expect(more).not.toBeNull()
    expect(more).toBeGreaterThan(0)
    expect(moreSamplesNeeded({ positive: 2, measured: 20 }, { positive: 2, measured: 20 })).toBeNull()
  })
})

describe('a replay', () => {
  it('reads the signal and counts the original call', () => {
    const base = adeBase()
    expect(tally(base)).toMatchObject({ ok: 20, positive: 3, negative: 17, unclear: 0 })
    expect(reproductionSentence(base)).toBe('Reproduced in 3 of 20 (15%). The original call appeared 3 times.')
    const quiet = reproduction('none', [reply(0, false, []), reply(1, false, [])])
    expect(reproductionSentence(quiet)).toBe('Not reproduced in 2 samples.')
  })

  it('compares a change and flags what else moved', () => {
    const result = compare(adeBase(), adeChange())
    expect(result.base).toEqual({ positive: 3, measured: 20 })
    expect(result.change).toEqual({ positive: 0, measured: 20 })
    expect(result.inconclusive).toBe(true)
    expect(result.sentence).toMatch(/^The signal dropped from 15% to 0% \(p = 0\.23\)\. This may still be chance; \d+ more samples per side would settle it\.$/)
    expect(result.sideEffects.map((effect) => effect.action)).toEqual(['coder::tree'])
    expect(result.sideEffects[0]).toMatchObject({ base: 7, change: 16 })
  })
})

describe('where the suggestion stands', () => {
  it('moves through the flow', () => {
    expect(standing(undefined).stage).toBe('setup')
    expect(standing(check).stage).toBe('ready')
    expect(standing(check, [reproduction('none', [], 'running')]).stage).toBe('reproducing')
    expect(standing(check, [adeBase()]).stage).toBe('reproduced')
    expect(standing(check, [reproduction('none', [reply(0, false, [])])]).stage).toBe('not_reproduced')
    expect(standing(check, [adeBase(), reproduction('custom', [], 'running')]).stage).toBe('testing')
    expect(standing(check, [adeBase(), adeChange()]).stage).toBe('compared')
    expect(standing(check, [adeBase(), reproduction('custom', [], 'failed')]).stage).toBe('failed')
  })

  it('labels the step and estimates the cost', () => {
    expect(stepLabel('e_t_4c79_4_assistant')).toBe('step 4')
    expect(estimateCost(0.05, 20)).toBeCloseTo(1)
    expect(estimateCost(undefined, 20)).toBeUndefined()
  })
})

describe('what a reproduction cost', () => {
  it('names the samples that reported no cost instead of counting them as free', () => {
    const base = reproduction('none', [])
    expect(reproductionCost({ ...base, cost_usd: 0.1 })).toBe('$0.10')
    expect(reproductionCost({ ...base, cost_usd: 0.1, cost_unknown_samples: 10 })).toBe('$0.10 + 10 not reported')
    expect(reproductionCost({ ...base, cost_unknown_samples: 20 })).toBe('20 not reported')
    expect(reproductionCost(base)).toBe('not reported')
  })
})
