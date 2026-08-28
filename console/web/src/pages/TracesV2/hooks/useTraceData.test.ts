import { describe, expect, it } from 'vitest'
import { shouldDeferTraceUpdate } from './useTraceData'

describe('shouldDeferTraceUpdate', () => {
  it('renders the first answer of an empty scope while hovered', () => {
    expect(shouldDeferTraceUpdate(true, false)).toBe(false)
  })

  it('freezes updates to an existing list while hovered', () => {
    expect(shouldDeferTraceUpdate(true, true)).toBe(true)
  })

  it('renders updates immediately when the list is not hovered', () => {
    expect(shouldDeferTraceUpdate(false, true)).toBe(false)
  })
})
