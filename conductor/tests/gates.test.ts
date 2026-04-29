import { describe, expect, it } from 'vitest'
import { allPassed } from '../src/gates.js'

describe('gates', () => {
  it('allPassed returns true on empty', () => {
    expect(allPassed({})).toBe(true)
  })

  it('allPassed returns true when every gate ok', () => {
    expect(allPassed({ a: { ok: true }, b: { ok: true } })).toBe(true)
  })

  it('allPassed returns false on any failure', () => {
    expect(allPassed({ a: { ok: true }, b: { ok: false, reason: 'tests failed' } })).toBe(false)
  })
})
