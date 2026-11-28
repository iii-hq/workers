import { type RefObject, useCallback, useEffect, useRef, useState } from 'react'

/** Where the question cards out of view sit, and which one a jump lands on. */
export interface OffscreenQuestions {
  /** How many waiting cards are out of view. */
  count: number
  /**
   * Message id of the card the next jump lands on: the waiting card after
   * the one the last jump landed on (wrapping to the first), or the oldest
   * one out of view before any jump.
   */
  target: string | null
  /** Which way the target is from the viewport. */
  direction: 'up' | 'down'
}

/** What the composer's notice can do with the waiting cards. */
export interface QuestionJumps {
  /** Land on the next waiting card in order (`OffscreenQuestions.target`). */
  next: () => void
  /** Land on one waiting card, by message id. */
  jumpTo: (messageId: string) => void
}

export const NO_OFFSCREEN_QUESTIONS: OffscreenQuestions = {
  count: 0,
  target: null,
  direction: 'up',
}

/** Part of a card that must show before it counts as seen: its whole height
 *  when shorter, else enough to read a question and reach its controls. */
const SEEN_PX = 64

interface Span {
  top: number
  bottom: number
}

function isSeen(span: Span | null, viewport: Span): boolean {
  if (!span) return false
  const shown =
    Math.min(span.bottom, viewport.bottom) - Math.max(span.top, viewport.top)
  const needed = Math.min(SEEN_PX, span.bottom - span.top)
  return needed > 0 && shown >= needed
}

/**
 * Which waiting cards are out of the viewport, and where the next jump goes.
 * `spanOf` measures a card's mounted node (`null` when it has none); a card
 * that cannot be measured counts as out of view, above.
 *
 * `after` is the card the previous jump landed on. Jumps walk the waiting
 * cards in transcript order from there, wrapping at the end, so each click
 * reaches the next card even though centring one scrolls the one before it
 * back out of view. Geometry alone would bounce between the first two.
 */
export function offscreenQuestions(
  ids: readonly string[],
  spanOf: (id: string) => Span | null,
  viewport: Span,
  after?: string | null,
): OffscreenQuestions {
  const spans = new Map(ids.map((id) => [id, spanOf(id)] as const))
  const hidden = ids.filter((id) => !isSeen(spans.get(id) ?? null, viewport))
  if (hidden.length === 0) return NO_OFFSCREEN_QUESTIONS
  const from = after ? ids.indexOf(after) : -1
  const target = from >= 0 ? ids[(from + 1) % ids.length] : hidden[0]
  const span = spans.get(target) ?? null
  return {
    count: hidden.length,
    target,
    direction: span && span.top > viewport.top ? 'down' : 'up',
  }
}

export function sameOffscreen(a: OffscreenQuestions, b: OffscreenQuestions) {
  return (
    a.count === b.count && a.target === b.target && a.direction === b.direction
  )
}

/** The card node of a message in the list (`data-message-id`, card level). */
export function questionCardNode(
  content: HTMLElement | null,
  id: string,
): HTMLElement | null {
  if (!content) return null
  for (const node of content.querySelectorAll<HTMLElement>(
    '[data-message-id]',
  )) {
    if (node.dataset.messageId === id) return node
  }
  return null
}

/**
 * Tracks which of `ids` (waiting question cards) are scrolled out of the
 * list's viewport. `check` re-measures on the next frame; call it from the
 * list's scroll handler. Layout changes (rows growing, the pane resizing)
 * re-measure on their own.
 */
export function useOffscreenQuestions(
  ids: readonly string[],
  containerRef: RefObject<HTMLElement | null>,
  contentRef: RefObject<HTMLElement | null>,
  enabled: boolean,
  after: string | null = null,
): { offscreen: OffscreenQuestions; check: () => void } {
  const [offscreen, setOffscreen] = useState(NO_OFFSCREEN_QUESTIONS)
  const idsRef = useRef(ids)
  idsRef.current = ids
  const enabledRef = useRef(enabled)
  enabledRef.current = enabled
  const afterRef = useRef(after)
  afterRef.current = after
  const frameRef = useRef<number | null>(null)

  const measure = useCallback(() => {
    frameRef.current = null
    const container = containerRef.current
    const content = contentRef.current
    const next =
      enabledRef.current && container && idsRef.current.length > 0
        ? offscreenQuestions(
            idsRef.current,
            (id) => {
              const node = questionCardNode(content, id)
              if (!node) return null
              const rect = node.getBoundingClientRect()
              return { top: rect.top, bottom: rect.bottom }
            },
            (() => {
              const box = container.getBoundingClientRect()
              return { top: box.top, bottom: box.bottom }
            })(),
            afterRef.current,
          )
        : NO_OFFSCREEN_QUESTIONS
    setOffscreen((current) => (sameOffscreen(current, next) ? current : next))
  }, [containerRef, contentRef])

  const check = useCallback(() => {
    if (frameRef.current !== null) return
    frameRef.current = requestAnimationFrame(measure)
  }, [measure])

  // biome-ignore lint/correctness/useExhaustiveDependencies: a change in the waiting set or the gate re-measures.
  useEffect(() => {
    check()
  }, [ids, enabled, after, check])

  useEffect(() => {
    const container = containerRef.current
    const content = contentRef.current
    if (!container || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(() => check())
    observer.observe(container)
    if (content) observer.observe(content)
    return () => observer.disconnect()
  }, [containerRef, contentRef, check])

  // Cleared, not just cancelled: a StrictMode remount reuses the ref, and a
  // stale id would make `check` think a frame is still coming.
  useEffect(
    () => () => {
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    },
    [],
  )

  return { offscreen, check }
}
