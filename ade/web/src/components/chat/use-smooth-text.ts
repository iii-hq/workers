import { useEffect, useRef, useState } from 'react'

/**
 * The reading pace, in characters per second: about what a fast model
 * streams, slow enough to read as being written, fast enough not to drag.
 */
const BASE_CPS = 320

/** A live stream never trails what has arrived by more than this. */
const STREAM_LAG_S = 1.2

/** A reply that is already whole is written out within about this long. */
const SETTLE_S = 2.2

/** Past this backlog (a reconnect, a transcript reload) nothing is paced. */
const SNAP_GAP = 24_000

/** A cut this close to the end of a word finishes the word. */
const WORD_REACH = 12

export type RevealMode = 'stream' | 'settle' | 'reveal'

/**
 * Where the revealed prefix may end, given a target boundary: never between
 * the halves of a surrogate pair, and never inside a run of backticks, so a
 * fence opening does not flash as inline code for a frame.
 */
export function safeCut(text: string, at: number): number {
  let cut = Math.min(at, text.length)
  if (cut <= 0) return 0
  const code = text.charCodeAt(cut - 1)
  if (code >= 0xd800 && code <= 0xdbff && cut < text.length) cut += 1
  if (text[cut - 1] === '`') {
    while (cut < text.length && text[cut] === '`') cut += 1
  }
  return cut
}

/**
 * How many characters per second the reveal moves at.
 *
 * - `stream`: tokens are still arriving. Hold the reading pace, and speed
 *   up only as far as it takes to stay within `STREAM_LAG_S` of the model,
 *   so a burst is spread out instead of dropped in.
 * - `settle` / `reveal`: the text is whole (the stream just ended, or the
 *   reply arrived in one piece). Pick one steady speed that writes the rest
 *   out within `SETTLE_S`, never slower than the reading pace, so long and
 *   short replies both finish promptly and neither crawls.
 */
export function revealSpeed(gap: number, mode: RevealMode): number {
  if (gap <= 0) return BASE_CPS
  const window = mode === 'stream' ? STREAM_LAG_S : SETTLE_S
  return Math.max(BASE_CPS, gap / window)
}

/**
 * Where the reveal lands after advancing `step` characters: the end of the
 * word it reached when that is close, so text appears a word at a time the
 * way tokens do rather than letter by letter.
 */
export function nextReveal(text: string, shown: number, step: number): number {
  const target = text.length
  if (shown >= target) return target
  if (target - shown > SNAP_GAP) return target
  let next = Math.min(target, shown + Math.max(1, Math.floor(step)))
  const limit = Math.min(target, next + WORD_REACH)
  while (next < limit && !/\s/.test(text[next])) next += 1
  return next
}

export interface SmoothText {
  /** The prefix to render. */
  text: string
  /** True while the prefix is still catching up with the content. */
  revealing: boolean
}

/**
 * Streamed text arrives in bursts of a few words at irregular intervals,
 * which reads as stutter. This returns a prefix of `content` that grows a
 * word or so per frame instead, so a reply appears to be written at one
 * steady reading pace. A reply that arrives whole can opt into the same treatment
 * with `revealOnMount`. With reduced motion the full text is returned as is.
 */
export function useSmoothText(
  content: string,
  streaming: boolean,
  reducedMotion = false,
  revealOnMount = false,
): SmoothText {
  const [paced] = useState(revealOnMount && !reducedMotion)
  const [, setShown] = useState(() => (paced ? 0 : content.length))
  const shownRef = useRef(paced ? 0 : content.length)
  const liveRef = useRef(streaming)
  // The pace only applies to text this mounted surface watched arrive. A
  // transcript that opens mid-reply shows what is there and smooths from
  // that point on.
  if (!liveRef.current && streaming && !paced) shownRef.current = content.length
  liveRef.current = streaming
  // Content that shrank (a different message under the same component, or
  // a reset) can never show a stale, longer prefix.
  if (shownRef.current > content.length) shownRef.current = content.length

  useEffect(() => {
    if (reducedMotion) {
      shownRef.current = content.length
      setShown(content.length)
      return
    }
    if (shownRef.current >= content.length) {
      setShown((current) =>
        current === content.length ? current : content.length,
      )
      return
    }
    const mode: RevealMode = streaming ? 'stream' : paced ? 'reveal' : 'settle'
    // Whole text moves at one speed for the rest of the run; a live stream
    // re-reads its backlog every frame as tokens land.
    const settledSpeed = revealSpeed(content.length - shownRef.current, mode)
    // Fractional characters carry over, so the pace holds at any frame rate.
    let carry = 0
    let last: number | null = null
    let frame = 0
    const tick = (now: number) => {
      const dt = last === null ? 1 / 60 : Math.min(0.1, (now - last) / 1000)
      last = now
      const gap = content.length - shownRef.current
      const speed = mode === 'stream' ? revealSpeed(gap, mode) : settledSpeed
      carry += speed * dt
      const step = Math.floor(carry)
      if (step > 0) {
        const next = safeCut(
          content,
          nextReveal(content, shownRef.current, step),
        )
        // A finished word is paid for out of the next frames' budget, so
        // word-at-a-time chunks still average out to the set speed.
        carry -= next - shownRef.current
        shownRef.current = next
        setShown(next)
      }
      if (shownRef.current < content.length) {
        frame = requestAnimationFrame(tick)
      }
    }
    frame = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(frame)
  }, [content, streaming, reducedMotion, paced])

  if (reducedMotion || typeof window === 'undefined') {
    return { text: content, revealing: false }
  }
  const cut = Math.min(shownRef.current, content.length)
  return cut >= content.length
    ? { text: content, revealing: false }
    : { text: content.slice(0, cut), revealing: true }
}
