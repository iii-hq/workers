import { describe, expect, it } from 'vitest'
import {
  isAtTail,
  nextTailScrollTop,
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
