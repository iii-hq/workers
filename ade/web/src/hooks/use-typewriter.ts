/**
 * A placeholder that types itself: each phrase is typed out, held, erased,
 * and the next one begins. The ⌘K palette uses it to teach its prefixes
 * (`>` commands, `#` files, `@` chats) without a sentence of instructions.
 *
 * Paused, it rests on the first phrase in full, so the field never shows a
 * half-typed word while it is not animating. Under `prefers-reduced-motion`
 * it never animates at all.
 */

import { useEffect, useState } from 'react'
import { REDUCED_MOTION_QUERY, useMediaQuery } from '@/hooks/use-media-query'

export interface TypewriterTiming {
  /** Per character while typing. */
  type: number
  /** Per character while erasing; faster than typing, the way people do. */
  erase: number
  /** How long a finished phrase stays readable. */
  hold: number
  /** The empty beat between one phrase and the next. */
  gap: number
}

export const TYPEWRITER_TIMING: TypewriterTiming = {
  type: 42,
  erase: 18,
  hold: 1800,
  gap: 320,
}

/** The text shown `elapsed` ms into a cycle through `phrases`. Pure, so the
 *  rhythm is testable without timers. */
export function typewriterFrame(
  phrases: readonly string[],
  elapsed: number,
  timing: TypewriterTiming = TYPEWRITER_TIMING,
): string {
  if (phrases.length === 0) return ''
  const lengths = phrases.map(
    (phrase) =>
      phrase.length * timing.type +
      timing.hold +
      phrase.length * timing.erase +
      timing.gap,
  )
  const cycle = lengths.reduce((sum, length) => sum + length, 0)
  let t = elapsed % cycle
  let index = 0
  while (t >= lengths[index]) {
    t -= lengths[index]
    index += 1
  }
  const phrase = phrases[index]
  const typing = phrase.length * timing.type
  if (t < typing) return phrase.slice(0, Math.floor(t / timing.type) + 1)
  t -= typing
  if (t < timing.hold) return phrase
  t -= timing.hold
  const erasing = phrase.length * timing.erase
  if (t < erasing)
    return phrase.slice(0, phrase.length - Math.floor(t / timing.erase) - 1)
  return ''
}

export function useTypewriter(
  phrases: readonly string[],
  active: boolean,
): string {
  const reduced = useMediaQuery(REDUCED_MOTION_QUERY)
  const animate = active && !reduced && phrases.length > 1
  const resting = phrases[0] ?? ''
  const [text, setText] = useState(resting)
  // Joined so a caller passing a fresh array literal each render does not
  // restart the cycle.
  const key = phrases.join('\u0000')

  // biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands in for `phrases`.
  useEffect(() => {
    if (!animate) {
      setText(resting)
      return
    }
    // The first phrase is already on screen when a pause ends, so the cycle
    // starts at its hold rather than typing it out again.
    const start = performance.now() - resting.length * TYPEWRITER_TIMING.type
    let frame = 0
    const tick = () => {
      const next = typewriterFrame(phrases, performance.now() - start)
      setText((current) => (current === next ? current : next))
      frame = window.requestAnimationFrame(tick)
    }
    frame = window.requestAnimationFrame(tick)
    return () => window.cancelAnimationFrame(frame)
  }, [animate, key, resting])

  return animate ? text : resting
}
