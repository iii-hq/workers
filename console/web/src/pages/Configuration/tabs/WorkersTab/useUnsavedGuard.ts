/**
 * Guard for unsaved configuration changes. Owned at the tab level so a
 * single dirty bit can intercept tab switches, in-tab selection changes,
 * and browser close/reload from one place.
 *
 * Surface area:
 *
 *   - `dirty` / `setDirty` — the boolean source of truth. Set by the
 *     `WorkerEditor` whenever its draft diverges from the loaded value.
 *   - `tryNavigate(action)` — wraps an in-app navigation. When dirty,
 *     prompts with `window.confirm` before running `action`. On confirm
 *     the dirty flag is cleared too (we're abandoning the draft, so the
 *     follow-up `setDirty(true)` from the newly-mounted editor is the
 *     thing that re-arms the bar).
 *
 * `beforeunload` is wired internally so the host doesn't have to remember
 * to attach it — the browser warning is the only line of defense against
 * accidental tab close, and forgetting it is a high-stakes oversight.
 */

import { useCallback, useEffect, useRef, useState } from 'react'

export interface UnsavedGuard {
  dirty: boolean
  setDirty: (dirty: boolean) => void
  /**
   * Run `action` after confirming with the operator when there are
   * unsaved changes. Clears the dirty flag on confirm so the next
   * editor mount starts clean. Returns `true` when the action ran and
   * `false` when the operator cancelled.
   */
  tryNavigate: (action: () => void) => boolean
}

export function useUnsavedGuard(): UnsavedGuard {
  const [dirty, setDirtyState] = useState(false)
  const dirtyRef = useRef(false)

  const setDirty = useCallback((next: boolean) => {
    dirtyRef.current = next
    setDirtyState(next)
  }, [])

  const tryNavigate = useCallback((action: () => void) => {
    if (dirtyRef.current) {
      const proceed = window.confirm(
        'discard unsaved changes? press cancel to keep editing.',
      )
      if (!proceed) return false
      dirtyRef.current = false
      setDirtyState(false)
    }
    action()
    return true
  }, [])

  // Browser-level guard: warn before tab close / reload while dirty.
  // The actual message is up to the browser (most ignore the string
  // these days and show a generic prompt) — what matters is returning
  // any string / truthy value at all to trigger the prompt.
  useEffect(() => {
    if (!dirty) return
    function onBeforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault()
      event.returnValue = ''
      return ''
    }
    window.addEventListener('beforeunload', onBeforeUnload)
    return () => window.removeEventListener('beforeunload', onBeforeUnload)
  }, [dirty])

  return { dirty, setDirty, tryNavigate }
}
