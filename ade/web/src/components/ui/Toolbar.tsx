import type { HTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'

export interface ToolbarProps extends HTMLAttributes<HTMLDivElement> {
  /** Trailing slot, pushed to the far end. */
  end?: ReactNode
}

/** Secondary toolbar strip: 36 px, raised. Name it with `aria-label`. */
export function Toolbar({ end, children, className, ...props }: ToolbarProps) {
  return (
    <div
      role="toolbar"
      className={cn(
        'flex h-9 shrink-0 items-center gap-1.5 bg-panel-raised px-2',
        className,
      )}
      {...props}
    >
      {children}
      {end != null ? (
        <div className="ml-auto flex items-center gap-1.5">{end}</div>
      ) : null}
    </div>
  )
}

/** Quiet status strip: 28 px, faint tabular text, same `end` slot. */
export function StatusBar({
  end,
  children,
  className,
  ...props
}: ToolbarProps) {
  return (
    <div
      className={cn(
        'flex h-7 shrink-0 items-center gap-2 px-2 font-sans text-[11px] text-ink-faint tabular-nums',
        className,
      )}
      {...props}
    >
      {children}
      {end != null ? (
        <div className="ml-auto flex items-center gap-2">{end}</div>
      ) : null}
    </div>
  )
}
