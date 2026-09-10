import { describe, expect, it } from 'vitest'
import { sameHiddenIdSet } from './trace-hidden-functions'

describe('sameHiddenIdSet', () => {
  it('treats equal contents as the same set, whatever the instances', () => {
    expect(sameHiddenIdSet(new Set(['a', 'b']), new Set(['b', 'a']))).toBe(true)
  })

  it('sees any difference in membership', () => {
    expect(sameHiddenIdSet(new Set(['a']), new Set(['a', 'b']))).toBe(false)
    expect(sameHiddenIdSet(new Set(['a', 'b']), new Set(['a', 'c']))).toBe(
      false,
    )
  })

  it('never matches a missing previous value', () => {
    expect(sameHiddenIdSet(undefined, new Set())).toBe(false)
  })
})
