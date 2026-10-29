import { describe, expect, it } from 'vitest'
import { nextReveal, revealSpeed, safeCut } from './use-smooth-text'

describe('revealSpeed', () => {
  it('holds the reading pace for a small backlog', () => {
    expect(revealSpeed(40, 'stream')).toBe(320)
    expect(revealSpeed(200, 'reveal')).toBe(320)
    expect(revealSpeed(0, 'settle')).toBe(320)
  })

  it('speeds a live stream up only enough to stay close behind', () => {
    expect(revealSpeed(1200, 'stream')).toBe(1000)
  })

  it('writes a long whole reply out within a couple of seconds', () => {
    expect(revealSpeed(4400, 'reveal')).toBeCloseTo(2000)
    expect(4400 / revealSpeed(4400, 'settle')).toBeCloseTo(2.2)
  })
})

describe('nextReveal', () => {
  const text = 'The quick brown fox jumps over the lazy dog'

  it('finishes the word it reaches', () => {
    expect(nextReveal(text, 0, 2)).toBe(3)
    expect(nextReveal(text, 3, 3)).toBe(9)
  })

  it('advances at least one character and never overshoots', () => {
    expect(nextReveal(text, 0, 0)).toBe(3)
    expect(nextReveal(text, 40, 20)).toBe(text.length)
    expect(nextReveal(text, text.length, 5)).toBe(text.length)
  })

  it('does not chase a very long word to its end', () => {
    const long = `a ${'x'.repeat(40)} b`
    expect(nextReveal(long, 0, 4)).toBe(16)
  })

  it('snaps a reconnect-sized backlog', () => {
    const huge = 'word '.repeat(6000)
    expect(nextReveal(huge, 0, 5)).toBe(huge.length)
  })
})

describe('safeCut', () => {
  it('keeps surrogate pairs together', () => {
    const text = 'a😀b'
    expect(safeCut(text, 2)).toBe(3)
    expect(safeCut(text, 3)).toBe(3)
  })

  it('does not stop inside a backtick run', () => {
    const text = 'run ```bash\nls'
    expect(safeCut(text, 5)).toBe(7)
    expect(safeCut(text, 6)).toBe(7)
    expect(safeCut(text, 8)).toBe(8)
  })

  it('clamps to the text', () => {
    expect(safeCut('abc', 10)).toBe(3)
    expect(safeCut('abc', 0)).toBe(0)
  })
})
