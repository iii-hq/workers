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
      onClick={onClick}
      aria-label={label}
      title={label}
      style={{
        border: '1px solid var(--color-rule)',
        background: 'transparent',
        color: 'var(--color-ink-faint)',
        width: 28,
        height: 28,
        cursor: 'pointer',
        fontFamily: 'inherit',
        fontSize: 14,
        lineHeight: 1,
      }}
    >
      ←
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
