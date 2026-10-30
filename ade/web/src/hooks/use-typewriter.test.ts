import { describe, expect, it } from 'vitest'
import { type TypewriterTiming, typewriterFrame } from './use-typewriter'

const T: TypewriterTiming = { type: 10, erase: 5, hold: 100, gap: 50 }
// "ab": 20 typing + 100 hold + 10 erasing + 50 gap = 180 per phrase.

describe('typewriterFrame', () => {
  it('types one character per step', () => {
    expect(typewriterFrame(['ab', 'cd'], 0, T)).toBe('a')
    expect(typewriterFrame(['ab', 'cd'], 10, T)).toBe('ab')
  })

  it('holds the whole phrase, then erases it', () => {
    expect(typewriterFrame(['ab', 'cd'], 60, T)).toBe('ab')
    expect(typewriterFrame(['ab', 'cd'], 120, T)).toBe('a')
    expect(typewriterFrame(['ab', 'cd'], 125, T)).toBe('')
  })

  it('rests empty in the gap, then starts the next phrase', () => {
    expect(typewriterFrame(['ab', 'cd'], 170, T)).toBe('')
    expect(typewriterFrame(['ab', 'cd'], 180, T)).toBe('c')
  })

  it('loops back to the first phrase', () => {
    expect(typewriterFrame(['ab', 'cd'], 360, T)).toBe('a')
  })

  it('shows nothing without phrases', () => {
    expect(typewriterFrame([], 42, T)).toBe('')
  })
})
