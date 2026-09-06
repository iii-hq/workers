/** Container-width hook + the two header glyphs the browser page and its
 * configuration editor share. */

import { useCallback, useRef, useState } from 'react'

/**
 * Container-driven responsive state for injected surfaces. The Console can
 * place worker UI inside panes of any width, so viewport media queries are
 * not a reliable signal for either the page or the configuration editor.
 */
export function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const ref = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return

      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)

      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) {
          setNarrow(next < threshold)
        }
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )

  return [ref, narrow]
}

/* ── inline icons ─────────────────────────────────────────────────────
 * Injected UI has no icon library to import — these are lucide's exact
 * 24×24 node data (lucide 1.25) at the page header's 1.5px stroke, sized
 * by the caller's className. All are decorative (aria-hidden); the
 * enclosing control carries the accessible name. (src/lib/icons.tsx keeps
 * the size-prop set the rest of the page and the chat cards use.) */

function iconProps(className?: string) {
  return {
    className,
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.5,
    strokeLinecap: 'round',
    strokeLinejoin: 'round',
  } as const
}

/** lucide `globe`: the browser worker's identity. */
export function GlobeIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <circle cx="12" cy="12" r="10" />
      <path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20" />
      <path d="M2 12h20" />
    </svg>
  )
}

/** lucide `chevron-left`: back, and forward when flipped. */
export function ChevronLeftIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="m15 18-6-6 6-6" />
    </svg>
  )
}
