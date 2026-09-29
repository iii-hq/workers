import type { HTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'

export interface ToolbarProps extends HTMLAttributes<HTMLElement> {
  /** Trailing slot, pushed to the far end. */
  end?: ReactNode
  /** Element to render: `form` for an address bar, `nav` for a rail. */
  as?: 'div' | 'form' | 'nav' | 'header' | 'footer' | 'section'
  /** A vertical toolbar is a 36 px wide rail; `end` sinks to the bottom. */
  orientation?: 'horizontal' | 'vertical'
}

/** Secondary toolbar strip: 36 px, raised. Name it with `aria-label`. */
export function Toolbar({
  end,
  children,
  className,
  as: Tag = 'div',
  orientation = 'horizontal',
  role = 'toolbar',
  ...props
}: ToolbarProps) {
  const vertical = orientation === 'vertical'
  return (
    <Tag
      role={role}
      aria-orientation={role === 'toolbar' && vertical ? 'vertical' : undefined}
      className={cn(
        'flex shrink-0 items-center gap-1.5 bg-panel-raised',
        vertical ? 'h-full w-9 flex-col py-2' : 'h-9 px-2',
        className,
      )}
      {...props}
    >
      {children}
      {end != null ? (
        <div
          className={cn(
            'flex items-center gap-1.5',
            vertical ? 'mt-auto flex-col' : 'ml-auto',
          )}
        >
          {end}
        </div>
      ) : null}
    </Tag>
  )
}

/** Quiet status strip: 28 px, faint tabular text, same `end` slot. */
export function StatusBar({
  end,
  children,
  className,
  as: Tag = 'div',
  orientation: _orientation,
  ...props
}: ToolbarProps) {
  return (
    <Tag
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
    </Tag>
  )
}
