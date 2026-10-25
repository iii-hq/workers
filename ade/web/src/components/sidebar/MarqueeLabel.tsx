import type { CSSProperties, ReactNode } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'
import './marquee-label.css'

/** Reading pace: how fast the text travels once it starts (px per second). */
const SPEED_PX_PER_S = 40
/** Room after the last glyph so the tail clears the right-edge fade. */
const END_PAD_PX = 16

interface MarqueeLabelProps {
  children: ReactNode
  className?: string
}

/**
 * A single-line label that reveals its overflow on hover: the text slides
 * right to left at reading pace until its end is in view, holds there, and
 * snaps back when the pointer leaves. At rest it fades out at the right
 * edge instead of ellipsising, so the fade is the tell that there is more.
 *
 * The hover itself is CSS (`.iii-ui-tree-item:hover`, fine pointers only,
 * and keyboard focus on the row); this component only measures how far the
 * text has to travel and hands that to the stylesheet. Transitions, not
 * keyframes, so a pointer passing over a row never commits it to a full
 * pass — the return is retargeted from wherever the text is.
 */
export function MarqueeLabel({ children, className }: MarqueeLabelProps) {
  const outer = useRef<HTMLSpanElement>(null)
  const inner = useRef<HTMLSpanElement>(null)
  const [distance, setDistance] = useState(0)

  const measure = useCallback(() => {
    const o = outer.current
    if (!o) return
    // The outer box's scroll width is the full text run even when the text
    // is clipped; the inner span is inline at rest and has no box of its own.
    const overflow = o.scrollWidth - o.clientWidth
    setDistance(overflow > 1 ? overflow + END_PAD_PX : 0)
  }, [])

  // Measured on mount, on every title change, and whenever the label's box
  // changes size — the sidebar resizes, and the row's hover actions take
  // width from the label when they appear.
  useEffect(() => {
    const o = outer.current
    if (!o || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(o)
    return () => observer.disconnect()
  }, [measure])
  // A new title inside the same box fires no resize: measure it directly.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `children` is the trigger, not an input
  useEffect(measure, [measure, children])

  const style =
    distance > 0
      ? ({
          '--marquee-distance': `${distance}px`,
          '--marquee-duration': `${Math.round((distance / SPEED_PX_PER_S) * 1000)}ms`,
        } as CSSProperties)
      : undefined

  return (
    <span
      ref={outer}
      className={cn('sidebar-marquee', className)}
      data-overflow={distance > 0 ? '' : undefined}
      style={style}
    >
      <span ref={inner} className="sidebar-marquee__text">
        {children}
      </span>
    </span>
  )
}
