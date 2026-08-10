/**
 * Click-to-copy, shared by every copy affordance in this UI: the op cards'
 * `RuntimeChip`, the family's `SandboxIdChip`, and the fleet page's
 * `CopyButton`.
 */

import { useCallback, useState } from 'react'

export type CopyState = 'idle' | 'copied' | 'failed'

/** How long the copied/failed flash stays up before reverting to idle. */
const FLASH_MS = 1400

async function copyText(text: string): Promise<boolean> {
  try {
    // Undefined outside a secure context (plain http:// over a LAN).
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch {
    /* fall through to the textarea path */
  }
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(ta)
    return ok
  } catch {
    return false
  }
}

/** `copy()` writes `text` and flashes the outcome; the caller renders
 *  `state` however its chrome wants. */
export function useCopyFlash(text: string): {
  state: CopyState
  copy: () => void
} {
  const [state, setState] = useState<CopyState>('idle')
  const copy = useCallback(() => {
    void copyText(text).then((ok) => {
      setState(ok ? 'copied' : 'failed')
      window.setTimeout(() => setState('idle'), FLASH_MS)
    })
  }, [text])
  return { state, copy }
}
