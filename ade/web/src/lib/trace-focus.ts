/**
 * "Show this trace": a one-shot request from anywhere in the tab (a
 * `@trace` mention, say) to the traces screen, which expands that trace.
 * The request waits for a traces screen that mounts after it was made, and
 * is consumed by the first one to take it.
 */

import { useEffect, useRef } from 'react'

interface FocusRequest {
  traceId: string
  seq: number
}

let pending: FocusRequest | null = null
let nextSeq = 1
const listeners = new Set<() => void>()

export function requestTraceFocus(traceId: string): void {
  const id = traceId.trim()
  if (!id) return
  pending = { traceId: id, seq: nextSeq++ }
  for (const listener of [...listeners]) listener()
}

/** Take the pending request, if any (it is then gone for everyone). */
export function takeTraceFocusRequest(): string | null {
  const request = pending
  pending = null
  return request?.traceId ?? null
}

/** Calls `onFocus` for a pending request now and for every later one. */
export function useTraceFocusRequest(onFocus: (traceId: string) => void): void {
  const onFocusRef = useRef(onFocus)
  onFocusRef.current = onFocus
  useEffect(() => {
    const deliver = () => {
      const traceId = takeTraceFocusRequest()
      if (traceId) onFocusRef.current(traceId)
    }
    deliver()
    listeners.add(deliver)
    return () => {
      listeners.delete(deliver)
    }
  }, [])
}
