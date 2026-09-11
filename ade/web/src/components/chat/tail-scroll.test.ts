import { describe, expect, it } from 'vitest'
import {
  isAtTail,
  nextTailScrollTop,
  scrollTopAfterPrepend,
  tailDistanceFromBottom,
  tailScrollTarget,
  tailStateAfterScroll,
} from './tail-scroll'

describe('tail scroll state', () => {
  it('calculates the reachable bottom for short and long content', () => {
    expect(
      tailScrollTarget({ scrollTop: 0, scrollHeight: 80, clientHeight: 100 }),
    ).toBe(0)
    expect(
      tailScrollTarget({ scrollTop: 0, scrollHeight: 640, clientHeight: 240 }),
    ).toBe(400)
  })

  it('pauses on any meaningful upward movement, even next to the bottom', () => {
    const metrics = { scrollTop: 399, scrollHeight: 500, clientHeight: 100 }

    expect(tailStateAfterScroll('following', 400, metrics)).toBe('paused')
  })

  it('keeps following when shrinking content clamps the viewport to its new tail', () => {
    const clampedToNewTail = {
      scrollTop: 380,
      scrollHeight: 480,
      clientHeight: 100,
    }

    expect(tailStateAfterScroll('following', 400, clampedToNewTail)).toBe(
      'following',
    )
    expect(tailStateAfterScroll('initializing', 400, clampedToNewTail)).toBe(
      'initializing',
    )
    expect(tailStateAfterScroll('paused', 400, clampedToNewTail)).toBe('paused')
  })

  it('does not resume until the viewport reaches the actual tail', () => {
    const away = { scrollTop: 397, scrollHeight: 500, clientHeight: 100 }
    const atTail = { ...away, scrollTop: 398 }

    expect(isAtTail(away)).toBe(false)
    expect(tailStateAfterScroll('paused', 396, away)).toBe('paused')
    expect(isAtTail(atTail)).toBe(true)
    expect(tailStateAfterScroll('paused', 397, atTail)).toBe('following')
  })

  it('does not finish initialization from its own instant scroll event', () => {
    const atTail = { scrollTop: 400, scrollHeight: 500, clientHeight: 100 }

    expect(tailStateAfterScroll('initializing', 400, atTail)).toBe(
      'initializing',
    )
  })

  it('keeps following when content grows beyond the old proximity threshold', () => {
    const metrics = { scrollTop: 400, scrollHeight: 900, clientHeight: 100 }

    expect(tailDistanceFromBottom(metrics)).toBe(400)
    expect(tailStateAfterScroll('following', 400, metrics)).toBe('following')
  })
})

describe('tail scroll glide', () => {
  it('moves monotonically toward a mutable target without overshooting', () => {
    const first = nextTailScrollTop(0, 100, 16)
    const retargeted = nextTailScrollTop(first, 180, 16)

    expect(first).toBeGreaterThan(0)
    expect(first).toBeLessThan(100)
    expect(retargeted).toBeGreaterThan(first)
    expect(retargeted).toBeLessThan(180)
  })

  it('settles tiny remaining distances exactly at the tail', () => {
    expect(nextTailScrollTop(99.25, 100, 16)).toBe(100)
  })
})

describe('scrollTopAfterPrepend', () => {
  /* A page of 900px landed above: the row the reader was looking at is now
     900px further down, so the offset moves by exactly that. */
  it('moves the offset by the height that appeared above', () => {
    expect(
      scrollTopAfterPrepend({ scrollTop: 120, scrollHeight: 2000 }, 2900),
    ).toBe(1020)
  })

  it('does nothing when nothing was added', () => {
    expect(
      scrollTopAfterPrepend({ scrollTop: 340, scrollHeight: 2000 }, 2000),
    ).toBe(340)
  })

  /* The reader who just opened the session sits at the bottom of a short
     first page: offset 0, list shorter than the viewport. Shifting by the
     page that landed above puts the offset at (or past, where the browser
     clamps it) the new bottom, so they stay where they were and the top
     sentinel ends up a whole page away instead of still in view. Skipping
     the shift for a following reader is what let one open pull in every
     page of history. */
  it('keeps a reader at the bottom of a short list at the bottom', () => {
    expect(
      scrollTopAfterPrepend({ scrollTop: 0, scrollHeight: 800 }, 1700),
    ).toBe(900)
  })

  /* Content that shrank (a taller viewport, a collapsed card) cannot push
     the offset below the top. */
  it('clamps at the top', () => {
    expect(
      scrollTopAfterPrepend({ scrollTop: 10, scrollHeight: 2000 }, 1500),
    ).toBe(0)
  })
})
