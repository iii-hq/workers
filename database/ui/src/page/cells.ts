/** Cell-value helpers shared by the result grid and the row inspector. */

import { useCallback, useEffect, useRef, useState } from 'react'

const COPIED_MS = 1200

/** Copyable string form of a cell value. */
export function cellText(value: unknown): string {
  if (value === null || value === undefined) return 'NULL'
  if (typeof value === 'object') {
    try {
      return JSON.stringify(value)
    } catch {
      return String(value)
    }
  }
  return String(value)
}

/** Best-effort clipboard write — the copy affordance is a convenience.
 *  clipboard may be missing (non-secure context) and writeText may reject
 *  (permission denied); the `?.catch` swallows both. */
export function copyText(text: string): void {
  void navigator.clipboard?.writeText(text)?.catch(() => {})
}

export interface CopyFeedback {
  /** Key of the field copied most recently, until the flash expires. */
  copied: string | null
  copy: (key: string, value: unknown) => void
}

/** Copy a value and flash "copied" against its key. The pending reset is
 *  cancelled on the next copy and on unmount, so a stale timer can neither
 *  clear a newer flash nor fire into a gone component. */
export function useCopyFeedback(): CopyFeedback {
  const [copied, setCopied] = useState<string | null>(null)
  const timer = useRef<number | undefined>(undefined)

  useEffect(() => () => window.clearTimeout(timer.current), [])

  const copy = useCallback((key: string, value: unknown) => {
    copyText(cellText(value))
    setCopied(key)
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setCopied(null), COPIED_MS)
  }, [])

  return { copied, copy }
}
