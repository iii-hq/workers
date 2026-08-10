/**
 * Minimal inline icon set for the fleet page — hand-rolled SVGs (no icon
 * dependency; the build keeps everything beyond react/console-ui bundled,
 * and a handful of paths do not earn a package). All render at `size` px in
 * `currentColor`, stroke style matching the console's icon weight.
 */

interface IconProps {
  size?: number
  className?: string
  'aria-hidden'?: boolean
}

function base(size: number) {
  return {
    width: size,
    height: size,
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.8,
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
  }
}

/** The sandbox glyph: a dashed-top cube — an isolated box. */
export function BoxIcon({ size = 16, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3z" strokeDasharray="3 2.4" />
      <path d="M12 12l8-4.5M12 12v9M12 12L4 7.5" />
    </svg>
  )
}

export function RefreshIcon({ size = 14, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <path d="M21 3v6h-6" />
    </svg>
  )
}

export function PlayIcon({ size = 14, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M7 4l13 8-13 8V4z" />
    </svg>
  )
}

export function PlusIcon({ size = 14, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M12 5v14M5 12h14" />
    </svg>
  )
}

export function FolderIcon({ size = 14, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M3 6a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6z" />
    </svg>
  )
}

export function FileIcon({ size = 14, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M6 3h8l4 4v14H6V3z" />
      <path d="M14 3v4h4" />
    </svg>
  )
}

export function ChevronIcon({ size = 12, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M9 6l6 6-6 6" />
    </svg>
  )
}

export function CopyIcon({ size = 13, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <rect x="9" y="9" width="12" height="12" rx="1.5" />
      <path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" />
    </svg>
  )
}

export function DownloadIcon({ size = 13, className, ...rest }: IconProps) {
  return (
    <svg {...base(size)} className={className} aria-hidden={rest['aria-hidden']}>
      <path d="M12 3v12m0 0l-5-5m5 5l5-5M4 21h16" />
    </svg>
  )
}
