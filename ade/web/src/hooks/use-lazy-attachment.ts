/**
 * The thumbnail of a stored picture, fetched when its chip scrolls into view.
 *
 * A transcript read no longer carries image bytes (see `fetchTranscript`), so
 * an image chip in a past message starts as a name, a size and an icon. The
 * bytes are worth fetching only for chips a person can see: a long
 * conversation has most of its screenshots far above the fold, and pulling
 * them all on open was the cost this replaces. `IntersectionObserver` says
 * when a chip is on screen; where there is none (a test runtime, an old
 * embedder) the fetch runs at once, which is the behaviour before this hook
 * existed and never a crash.
 *
 * The fetch itself goes through `attachment-cache`, so a chip that remounts
 * — the list virtualising past it, a session switched away and back — draws
 * its thumbnail on the first paint and never asks the store twice.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  loadAttachment,
  peekAttachment,
} from '@/lib/attachments/attachment-cache'

/** Retries a click may spend after the first fetch failed. */
export const LAZY_ATTACHMENT_RETRIES = 1

export type LazyAttachmentState =
  | { phase: 'pending' }
  | { phase: 'loading' }
  | { phase: 'ready'; dataUrl: string }
  | { phase: 'failed'; reason: string; retryable: boolean }

export interface LazyAttachment {
  /** Attach to the element whose visibility should start the fetch. */
  ref: (element: Element | null) => void
  state: LazyAttachmentState
  /**
   * Fetch now: before the chip has scrolled in (a click on the placeholder),
   * or again after a failure while a retry is left. A no-op otherwise.
   */
  request: () => void
}

/**
 * Call `onVisible` once, the first time `element` intersects the viewport,
 * and stop watching. Returns the disconnect for the effect cleanup. Without
 * an observer implementation the callback runs immediately: an image that
 * loads too early is a wasted request, one that never loads is a broken chip.
 */
export function observeFirstIntersection(
  element: Element,
  onVisible: () => void,
): () => void {
  const Observer = (
    globalThis as { IntersectionObserver?: typeof IntersectionObserver }
  ).IntersectionObserver
  if (typeof Observer !== 'function') {
    onVisible()
    return () => {}
  }
  let fired = false
  const observer = new Observer((records) => {
    if (fired || !records.some((r) => r.isIntersecting)) return
    fired = true
    observer.disconnect()
    onVisible()
  })
  observer.observe(element)
  return () => observer.disconnect()
}

export function useLazyAttachment(
  sessionId: string | undefined,
  attachmentId: string | undefined,
  enabled: boolean,
): LazyAttachment {
  const key =
    enabled && sessionId && attachmentId
      ? `${sessionId}/${attachmentId}`
      : undefined
  const [state, setState] = useState<LazyAttachmentState>(() => {
    // A second mount finds the bytes already here and skips the placeholder;
    // reading synchronously is what avoids a flash of icon before the image.
    const cached =
      sessionId && attachmentId && key
        ? peekAttachment(sessionId, attachmentId)
        : undefined
    return cached
      ? { phase: 'ready', dataUrl: cached.dataUrl }
      : { phase: 'pending' }
  })
  const [element, setElement] = useState<Element | null>(null)
  const retries = useRef(0)
  // Which attachment a fetch was started for, if any. A ref rather than the
  // state so the observer effect below does not re-arm on every state change
  // and start a second fetch; and a chip re-keyed to another attachment while
  // one is in flight must not show the old picture when it lands.
  const started = useRef<string | undefined>(
    state.phase === 'ready' ? key : undefined,
  )

  const load = useCallback(() => {
    if (!key || !sessionId || !attachmentId) return
    started.current = key
    setState({ phase: 'loading' })
    loadAttachment(sessionId, attachmentId).then(
      (cached) => {
        if (started.current !== key) return
        setState({ phase: 'ready', dataUrl: cached.dataUrl })
      },
      (err: unknown) => {
        if (started.current !== key) return
        setState({
          phase: 'failed',
          reason: err instanceof Error ? err.message : String(err),
          retryable: retries.current < LAZY_ATTACHMENT_RETRIES,
        })
      },
    )
  }, [key, sessionId, attachmentId])

  useEffect(() => {
    if (!key || !element || started.current === key) return
    // Checked again at fire time: a click on the placeholder may have
    // started the fetch while the observer was still waiting.
    return observeFirstIntersection(element, () => {
      if (started.current !== key) load()
    })
  }, [key, element, load])

  const request = useCallback(() => {
    if (state.phase === 'pending') {
      load()
      return
    }
    if (state.phase !== 'failed' || !state.retryable) return
    retries.current += 1
    load()
  }, [state, load])

  return { ref: setElement, state, request }
}
