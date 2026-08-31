/** Small shared UI pieces used across the page views. */

import { useCallback, useState } from 'react'

export function LiveDot() {
  return (
    <span
      className="state-ui-live"
      title="live — subscribed to the state trigger type; created/updated/deleted events stream in"
    >
      <span className="dot">●</span> live
    </span>
  )
}

/** Narrow-mode drill-out affordance (scopes ← keys ← value). */
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
      className="state-ui-back"
      onClick={onClick}
      aria-label={label}
      title={label}
    >
      <ChevronLeftIcon className="state-ui-back-icon" />
    </button>
  )
}

/** Column-head refresh affordance. */
export function RefreshButton({
  onClick,
  label,
}: {
  onClick: () => void
  label: string
}) {
  return (
    <button
      type="button"
      className="state-ui-iconbtn"
      onClick={onClick}
      aria-label={label}
      title={label}
    >
      <RotateIcon className="state-ui-iconbtn-icon" />
    </button>
  )
}

/** Transient per-row highlight for live-arrived changes. */
export function useFlash(): [ReadonlySet<string>, (k: string) => void] {
  const [flashed, setFlashed] = useState<ReadonlySet<string>>(new Set())
  const mark = useCallback((k: string) => {
    setFlashed((prev) => new Set(prev).add(k))
    window.setTimeout(() => {
      setFlashed((prev) => {
        const next = new Set(prev)
        next.delete(k)
        return next
      })
    }, 1400)
  }, [])
  return [flashed, mark]
}

/* ── inline icons ─────────────────────────────────────────────────────
 * Injected UI has no icon library to import — these are hand-inlined
 * 24×24 stroke glyphs (lucide geometry: 1.5px stroke, round caps) sized
 * by the caller's className. All are decorative (aria-hidden); the
 * enclosing control carries the accessible name. */

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

/** Stacked-cylinders glyph: the key–value store identity. */
export function DatabaseIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <ellipse cx="12" cy="5" rx="9" ry="3" />
      <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
      <path d="M3 12c0 1.66 4 3 9 3s9-1.34 9-3" />
    </svg>
  )
}

export function ChevronLeftIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="m15 18-6-6 6-6" />
    </svg>
  )
}

export function RotateIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M21 12a9 9 0 1 1-2.64-6.36L21 8" />
      <path d="M21 3v5h-5" />
    </svg>
  )
}
