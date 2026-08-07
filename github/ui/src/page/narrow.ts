/**
 * Container-driven layout switching shared by both page views. The console
 * can host the page in panes of any size (splits, phones, narrow columns),
 * so layout decisions key off the width the view actually HAS — a
 * ResizeObserver on the view's own root — never a viewport media query.
 */

import { useCallback, useRef, useState } from 'react'

/** Container width (px) below which a view collapses to its narrow layout
 * (single-line rows regroup, the commit detail becomes a drill-in). Chosen
 * from the wide layouts' real minimum: the activity grid's four columns and
 * the graph row's gutter + refs + subject + meta both stop fitting around
 * here. */
export const NARROW_BELOW = 720

/** Observe the view root's own width. Returns a callback ref to put on the
 * root plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash;
 * zero widths (display:none) are ignored so a hidden view keeps its last
 * real layout. */
export function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [refCb, narrow]
}
