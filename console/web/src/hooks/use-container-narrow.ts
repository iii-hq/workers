import { useCallback, useRef, useState } from 'react'

/**
 * Observe an element's own width and report whether it is currently
 * narrower than `threshold`. Container-driven (a ResizeObserver on the
 * node, not a viewport media query) so the same component adapts inside
 * any pane the console gives it — a phone viewport and a squeezed tab
 * column behave identically.
 *
 * Measures synchronously when the ref attaches to avoid a wide-mode
 * flash; zero widths (display:none hosts) are ignored so a hidden pane
 * keeps its last real layout.
 */
export function useContainerNarrow(
  threshold: number,
): [(node: HTMLElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLElement | null) => {
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
