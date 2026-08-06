/**
 * Small presentational pieces shared by the function-trigger views and the
 * page. Ports of the console's sandbox/directory card chrome, restyled
 * with `dir-ui-*` classes (styles.css, scoped under
 * `[data-iii-ui="iii-directory"]`) — injected UI can't lean on the
 * console's Tailwind utility output.
 */

import { Badge } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'

export function Chip({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={`dir-ui-chip${className ? ` ${className}` : ''}`}>{children}</span>
}

/** Two-tone chip with a small uppercase label and a value. */
export function KvChip({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Chip>
      <span className="k">{label}</span>
      <span className="v">{children}</span>
    </Chip>
  )
}

export function MetaRow({ children }: { children: ReactNode }) {
  return <div className="dir-ui-meta">{children}</div>
}

export function StatusPill({
  label,
  variant = 'default',
}: {
  label: string
  variant?: 'default' | 'warn' | 'alert' | 'accent'
}) {
  return (
    <Badge variant={variant} className="dir-ui-pill-flat">
      {label}
    </Badge>
  )
}

export function ActionLine({
  symbol,
  children,
  tone = 'accent',
}: {
  symbol: string
  children: ReactNode
  tone?: 'accent' | 'warn' | 'ink'
}) {
  return (
    <div className="dir-ui-action">
      <span className={`sym tone-${tone}`}>{symbol}</span>
      <div className="body">{children}</div>
    </div>
  )
}

/** The card wrapper every settled/running view sits in. */
export function Card({ children }: { children: ReactNode }) {
  return <div className="dir-ui-card">{children}</div>
}

export function SectionHead({ children }: { children: ReactNode }) {
  return <div className="dir-ui-section-head">{children}</div>
}

export function SubHead({ children }: { children: ReactNode }) {
  return <div className="dir-ui-subhead">{children}</div>
}

export function EmptyRow({ label }: { label: string }) {
  return <div className="dir-ui-empty">· {label}</div>
}

/** Narrow-mode drill-out affordance (the state worker's ← pattern). */
export function BackButton({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button type="button" className="dir-ui-back" onClick={onClick} aria-label={label} title={label}>
      <ChevronLeftIcon className="dir-ui-back-icon" />
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

/** Document-with-folded-corner + "M" tick: the markdown file identity. */
export function MarkdownFileIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M14 3v5h5" />
      <path d="M6 3h8l5 5v12a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" />
      <path d="M9 17v-4l2 2 2-2v4" />
    </svg>
  )
}

export function SearchIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </svg>
  )
}

export function XIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
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

export function PulseLine({ label }: { label: string }) {
  return <div className="dir-ui-pulse">· {label}</div>
}
