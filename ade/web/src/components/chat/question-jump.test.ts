import { describe, expect, it } from 'vitest'
import { NO_OFFSCREEN_QUESTIONS, offscreenQuestions } from './question-jump'

const viewport = { top: 100, bottom: 700 }

describe('offscreenQuestions', () => {
  it('is empty when every waiting card shows', () => {
    const spans = { a: { top: 200, bottom: 400 }, b: { top: 650, bottom: 900 } }
    // `b` shows its first 50px of a 250px card: under 64px, so not yet seen.
    expect(
      offscreenQuestions(['a'], (id) => spans[id as 'a'] ?? null, viewport),
    ).toBe(NO_OFFSCREEN_QUESTIONS)
    expect(
      offscreenQuestions(['a', 'b'], (id) => spans[id as 'a'], viewport),
    ).toEqual({ count: 1, target: 'b', direction: 'down' })
  })

  it('a short card counts as seen once all of it shows', () => {
    expect(
      offscreenQuestions(['a'], () => ({ top: 660, bottom: 700 }), viewport),
    ).toBe(NO_OFFSCREEN_QUESTIONS)
  })

  it('targets the oldest card out of view, above the viewport', () => {
    const spans: Record<string, { top: number; bottom: number }> = {
      old: { top: -900, bottom: -600 },
      seen: { top: 300, bottom: 500 },
      below: { top: 800, bottom: 1000 },
    }
    expect(
      offscreenQuestions(['old', 'seen', 'below'], (id) => spans[id], viewport),
    ).toEqual({ count: 2, target: 'old', direction: 'up' })
  })

  it('a card it cannot measure (unmounted, folded) counts as above', () => {
    expect(offscreenQuestions(['x'], () => null, viewport)).toEqual({
      count: 1,
      target: 'x',
      direction: 'up',
    })
  })

  it('walks the waiting cards in order from the last landing, wrapping at the end', () => {
    // All three are above the viewport: centring one scrolls the others out.
    const above = () => ({ top: -900, bottom: -700 })
    const ids = ['a', 'b', 'c']
    expect(offscreenQuestions(ids, above, viewport).target).toBe('a')
    expect(offscreenQuestions(ids, above, viewport, 'a').target).toBe('b')
    expect(offscreenQuestions(ids, above, viewport, 'b').target).toBe('c')
    expect(offscreenQuestions(ids, above, viewport, 'c').target).toBe('a')
  })

  it('goes on to the next card even when that one is in view', () => {
    const spans: Record<string, { top: number; bottom: number }> = {
      a: { top: -900, bottom: -700 },
      b: { top: 300, bottom: 500 },
    }
    expect(
      offscreenQuestions(['a', 'b'], (id) => spans[id], viewport, 'a'),
    ).toEqual({ count: 1, target: 'b', direction: 'down' })
  })
})
