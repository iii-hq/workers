/**
 * Small shared page widgets: the narrow-mode drill-out affordance and the
 * dirty-delta reporter every in-place editor uses to join the page-level
 * unsaved-changes guard.
 */

import { useEffect } from 'react'
import { ChevronLeft } from './icons'

/** Narrow-mode drill-out affordance (banks ← workspace). */
export function BackButton({
  onClick,
  label,
}: {
  onClick: () => void
  label: string
}) {
  return (
    <button
      type="button"
      className="mem-ui-back"
      onClick={onClick}
      aria-label={label}
      title={label}
    >
      <ChevronLeft size={16} aria-hidden />
    </button>
  )
}

/**
 * Report a boolean dirty flag up as +1/−1 deltas. Several editors can be
 * dirty at once (every rule has its own editor, every memory row can be
 * mid-edit), so the page guards navigation with a counter, not a boolean —
 * one editor saving must not release the guard for the others. The effect
 * cleanup also runs on unmount, so a discarded editor always releases its
 * slot. `report` must be referentially stable (the page's useCallback).
 */
export function useDirtyDelta(
  dirty: boolean,
  report: (delta: number) => void,
) {
  useEffect(() => {
    if (!dirty) return
    report(1)
    return () => report(-1)
  }, [dirty, report])
}
