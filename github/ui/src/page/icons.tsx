/**
 * Small inline SVG icon set — the injected page can't pull in lucide-react
 * (nothing is bundled but zod), so the handful of glyphs the ported components
 * used are hand-drawn here. Every icon inherits `currentColor` and takes a
 * numeric `size` (Tailwind `w-*`/`h-*` classes don't apply in injected UI).
 * `className` is accepted so components like the console's `EmptyState`, which
 * pass one, type-check.
 */

import type { CSSProperties, ReactNode } from 'react'

export interface IconProps {
  size?: number
  className?: string
  style?: CSSProperties
  'aria-hidden'?: boolean
  'aria-label'?: string
}

function Svg({
  size = 14,
  children,
  className,
  style,
  'aria-hidden': ariaHidden,
  'aria-label': ariaLabel,
  ...rest
}: IconProps & { children: ReactNode }) {
  // Decorative by default. A caller passing aria-label opts into a labeled,
  // non-hidden icon (role=img so it reads as a graphic); an explicit
  // aria-hidden always wins.
  const labeled = ariaLabel != null
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      style={{ flexShrink: 0, display: 'inline-block', ...style }}
      role={labeled ? 'img' : undefined}
      aria-label={ariaLabel}
      aria-hidden={ariaHidden ?? (labeled ? undefined : true)}
      {...rest}
    >
      {children}
    </svg>
  )
}

export function RefreshCw(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
      <path d="M3 21v-5h5" />
    </Svg>
  )
}

export function AlertCircle(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="10" />
      <path d="M12 8v4M12 16h.01" />
    </Svg>
  )
}

export function FolderGit2(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v3" />
      <circle cx="13" cy="19" r="2" />
      <path d="M15 19h5a2 2 0 0 0 2-2v-2" />
      <circle cx="20" cy="13" r="2" />
    </Svg>
  )
}

export function Activity(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
    </Svg>
  )
}
