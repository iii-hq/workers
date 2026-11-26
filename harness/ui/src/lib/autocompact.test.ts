import { describe, expect, it } from 'vitest'
import {
  AT_MODEL_LIMIT,
  autocompactMark,
  COMPACTS_AFTER_TURN,
  chipLabel,
  compactsAfterTurn,
  meterLabel,
  meterLine,
  parseAutocompact,
  ruleName,
  sizeWord,
  tickRatio,
} from './autocompact'

const project = { scope: 'project' as const, project_id: 'p1', project_name: 'acme-web' }

describe('parseAutocompact', () => {
  it('reads contract C and drops what it does not know', () => {
    expect(
      parseAutocompact({
        setting_tokens: 160_000,
        effective_tokens: 160_000,
        cap_tokens: 116_000,
        rule: { ...project, extra: true },
      }),
    ).toEqual({ setting_tokens: 160_000, effective_tokens: 160_000, cap_tokens: 116_000, rule: project })
  })

  it('is null from an older harness and tolerant of a malformed one', () => {
    expect(parseAutocompact(undefined)).toBeNull()
    expect(parseAutocompact(null)).toBeNull()
    expect(parseAutocompact([1])).toBeNull()
    expect(parseAutocompact({ effective_tokens: 'lots', rule: { scope: 'team' } })).toEqual({
      setting_tokens: null,
      effective_tokens: null,
      cap_tokens: null,
      rule: null,
    })
  })
})

describe('autocompactMark', () => {
  it('marks the effective size below usable', () => {
    expect(
      autocompactMark({ usable: 916_000, autocompact: { effective_tokens: 400_000, rule: { scope: 'global' } } }),
    ).toEqual({ effective: 400_000, rule: { scope: 'global' } })
  })

  it('has no size at the model limit, nor at or past usable', () => {
    expect(autocompactMark({ usable: 116_000, autocompact: { setting_tokens: 160_000, effective_tokens: null } })).toEqual(
      { effective: null, rule: null },
    )
    expect(autocompactMark({ usable: 116_000, autocompact: { effective_tokens: 116_000 } })?.effective).toBeNull()
  })

  it('is null with no autocompact on the snapshot', () => {
    expect(autocompactMark({ usable: 200_000 })).toBeNull()
  })
})

describe('tickRatio', () => {
  it('places the tick at effective over usable', () => {
    expect(tickRatio(400_000, 916_000)).toBeCloseTo(0.4367, 3)
    expect(tickRatio(160_000, 200_000)).toBe(0.8)
  })
  it('has no tick without a size or a usable', () => {
    expect(tickRatio(null, 200_000)).toBeNull()
    expect(tickRatio(undefined, 200_000)).toBeNull()
    expect(tickRatio(100_000, 0)).toBeNull()
  })
})

describe('words', () => {
  it('writes sizes as the ADE does', () => {
    expect(sizeWord(9_500)).toBe('9.5k')
    expect(sizeWord(160_000)).toBe('160k')
    expect(sizeWord(161_400)).toBe('161k')
    expect(sizeWord(1_000_000)).toBe('1M')
    expect(sizeWord(1_200_000)).toBe('1.2M')
  })

  it('names the rule', () => {
    expect(ruleName({ scope: 'global' })).toBe('global rule')
    expect(ruleName(project)).toBe('acme-web rule')
    expect(ruleName({ scope: 'session' })).toBe("this conversation's rule")
  })

  it('writes the label and the line', () => {
    expect(meterLabel(400_000)).toBe('auto 400k')
    expect(meterLine(160_000, project)).toBe('Auto-compact at 160k · acme-web rule')
    expect(meterLine(160_000, null)).toBe('Auto-compact at 160k')
    expect(meterLine(null, project)).toBe(AT_MODEL_LIMIT)
    expect(AT_MODEL_LIMIT).toBe("At the model's limit")
  })
})

describe('Compacts after this turn', () => {
  const mark = { effective: 160_000, rule: project }

  it('reads so while a running turn is at or past the size', () => {
    expect(compactsAfterTurn(160_000, 160_000, true)).toBe(true)
    expect(compactsAfterTurn(171_000, 160_000, true)).toBe(true)
    expect(chipLabel(171_000, mark, true)).toBe(COMPACTS_AFTER_TURN)
    expect(COMPACTS_AFTER_TURN).toBe('Compacts after this turn')
  })

  it('is the plain label below the size, between turns, or on an older console', () => {
    expect(chipLabel(120_000, mark, true)).toBe('auto 160k')
    expect(chipLabel(171_000, mark, false)).toBe('auto 160k')
    expect(chipLabel(171_000, mark, undefined)).toBe('auto 160k')
  })

  it('has no label at the model limit or without autocompact', () => {
    expect(chipLabel(171_000, { effective: null, rule: null }, true)).toBeNull()
    expect(chipLabel(171_000, null, true)).toBeNull()
    expect(compactsAfterTurn(171_000, null, true)).toBe(false)
  })
})
