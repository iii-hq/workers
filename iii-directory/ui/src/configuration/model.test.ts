import { describe, expect, it } from 'vitest'
import { booleanWithDefault } from './model'

describe('booleanWithDefault', () => {
  it('uses the worker default when a migrated value omits the field', () => {
    expect(booleanWithDefault(undefined, false)).toBe(false)
    expect(booleanWithDefault(undefined, true)).toBe(true)
  })

  it('preserves explicit boolean values', () => {
    expect(booleanWithDefault(true, false)).toBe(true)
    expect(booleanWithDefault(false, true)).toBe(false)
  })
})
