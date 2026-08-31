/** Small shared UI pieces + inline icons for the computer page. */

export function LivePill({ live }: { live: boolean }) {
  return (
    <span
      className={`cp-ui-live${live ? ' is-live' : ''}`}
      title={
        live
          ? 'live — subscribed to the session lifecycle triggers; the rail updates as sessions start and stop'
          : 'polling — lifecycle trigger bindings unavailable; the session list refreshes every 10s'
      }
    >
      <span className="dot">●</span> {live ? 'live' : 'polling'}
    </span>
  )
}

/** Narrow-mode drill-out affordance (session list ← viewport). */
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
      className="cp-ui-back"
      onClick={onClick}
      aria-label={label}
      title={label}
    >
      <ChevronLeftIcon className="cp-ui-back-icon" />
    </button>
  )
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

/** Desktop-display glyph: the computer worker's identity. */
export function MonitorIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <rect x="2" y="3" width="20" height="14" rx="2" />
      <path d="M8 21h8" />
      <path d="M12 17v4" />
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
