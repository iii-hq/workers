/** Small shared UI pieces + inline icons for the browser page. */

import { StatusDot } from '@iii-dev/console-ui'

/** Header live / polling indicator for the session feed. */
export function LivePill({ live }: { live: boolean }) {
  return (
    <span
      className="br-ui-live"
      title={
        live
          ? 'live — updates arrive on the browser session triggers'
          : 'polling — live bindings unavailable; refreshing on a timer'
      }
    >
      <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
      {live ? 'live' : 'polling'}
    </span>
  )
}

/** Narrow-mode drill-out affordance (session list ← workspace). */
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
      className="br-ui-back"
      onClick={onClick}
      aria-label={label}
      title={label}
    >
      <ChevronLeftIcon className="br-ui-back-icon" />
    </button>
  )
}

/** Column-head refresh affordance. */
export function RefreshButton({
  onClick,
  label,
  disabled,
  spinning,
}: {
  onClick: () => void
  label: string
  disabled?: boolean
  spinning?: boolean
}) {
  return (
    <button
      type="button"
      className="br-ui-iconbtn"
      onClick={onClick}
      aria-label={label}
      title={label}
      disabled={disabled}
    >
      <RotateIcon
        className={`br-ui-iconbtn-icon${spinning ? ' br-ui-spin' : ''}`}
      />
    </button>
  )
}

/* ── inline icons ─────────────────────────────────────────────────────
 * Injected UI has no icon library to import — these are hand-inlined
 * 24×24 stroke glyphs (lucide geometry: 1.5px stroke, round caps) sized
 * by the caller's className. All are decorative (aria-hidden); the
 * enclosing control carries the accessible name. (src/lib/icons.tsx keeps
 * the older size-prop set the chat cards and rail rows use.) */

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

/** Globe glyph: the browser worker's identity. */
export function GlobeIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <circle cx="12" cy="12" r="10" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10" />
      <path d="M2 12h20" />
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
