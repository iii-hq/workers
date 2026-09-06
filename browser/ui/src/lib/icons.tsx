/**
 * Inline lucide icons for the injected UI. Nothing but zod is bundled, so
 * the console's `lucide-react` is not importable here; each glyph below is
 * the exact node data of the lucide icon it is named after (lucide 1.25,
 * 24×24 grid, 2px round stroke), copied rather than approximated so the
 * browser page draws the same icons as the rest of the console. Every icon
 * inherits `currentColor` and takes a numeric `size` (Tailwind `w-*`/`h-*`
 * classes don't apply in injected UI). `className` is accepted so
 * components like the console's `EmptyState`, which pass one, type-check.
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
  size = 16,
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

/** lucide `globe`: a regular tab's favicon slot and the address bar. */
export function Globe(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="10" />
      <path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20" />
      <path d="M2 12h20" />
    </Svg>
  )
}

/** lucide `plus`. */
export function Plus(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5 12h14" />
      <path d="M12 5v14" />
    </Svg>
  )
}

/** lucide `refresh-cw`: reload. */
export function RefreshCw(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
      <path d="M8 16H3v5" />
    </Svg>
  )
}

/** lucide `message-square-plus`: annotate. */
export function MessageSquarePlus(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z" />
      <path d="M12 8v6" />
      <path d="M9 11h6" />
    </Svg>
  )
}

/** lucide `x`: close. */
export function X(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </Svg>
  )
}

/** lucide `external-link`: open the page in your own browser. */
export function ExternalLink(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M15 3h6v6" />
      <path d="M10 14 21 3" />
      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
    </Svg>
  )
}

/** lucide `ellipsis-vertical`: the browser menu. */
export function MoreVertical(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="1" />
      <circle cx="12" cy="5" r="1" />
      <circle cx="12" cy="19" r="1" />
    </Svg>
  )
}

/** lucide `search`. */
export function Search(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m21 21-4.34-4.34" />
      <circle cx="11" cy="11" r="8" />
    </Svg>
  )
}

/** lucide `chevron-up`. */
export function ChevronUp(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m18 15-6-6-6 6" />
    </Svg>
  )
}

/** lucide `chevron-down`. */
export function ChevronDown(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m6 9 6 6 6-6" />
    </Svg>
  )
}

/** lucide `minus`. */
export function Minus(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5 12h14" />
    </Svg>
  )
}

/** lucide `download`. */
export function Download(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M12 15V3" />
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="m7 10 5 5 5-5" />
    </Svg>
  )
}

/** lucide `hat-glasses`: an incognito tab. */
export function Incognito(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M14 18a2 2 0 0 0-4 0" />
      <path d="m19 11-2.11-6.657a2 2 0 0 0-2.752-1.148l-1.276.61A2 2 0 0 1 12 4H8.5a2 2 0 0 0-1.925 1.456L5 11" />
      <path d="M2 11h20" />
      <circle cx="17" cy="18" r="3" />
      <circle cx="7" cy="18" r="3" />
    </Svg>
  )
}

/** lucide `moon`: a tab asleep (page closed, tab kept). */
export function Moon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401" />
    </Svg>
  )
}
