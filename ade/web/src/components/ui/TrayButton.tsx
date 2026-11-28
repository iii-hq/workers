import type { ComponentProps, ReactNode } from 'react'
import { cn } from '@/lib/utils'

/**
 * The tray above the composer (the queued-message strip's surface): one
 * rounded block that holds `TrayButton` rows. Callers pass layout only.
 */
export function Tray({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="tray"
      className={cn('overflow-hidden rounded-xl bg-surface', className)}
      {...props}
    />
  )
}

/**
 * A full-width row in a `Tray`: a leading icon, its words, and a trailing
 * hint such as "Next ↑". The whole row is the control. 44px tall in a
 * narrow pane, 36px once the pane is wide; long words wrap. Callers pass
 * layout only.
 */
export function TrayButton({
  icon,
  trailing,
  className,
  children,
  type = 'button',
  ...props
}: ComponentProps<'button'> & {
  icon?: ReactNode
  trailing?: ReactNode
}) {
  return (
    <button
      type={type}
      data-slot="tray-button"
      className={cn(
        'flex min-h-11 w-full min-w-0 items-center gap-2 px-3 py-2 text-start text-base text-ink transition-colors duration-(--motion-duration-control) ease-(--motion-ease-standard) hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none focus-visible:ring-inset @2xl:min-h-9 @2xl:py-1.5 @2xl:text-sm',
        className,
      )}
      {...props}
    >
      {icon ? (
        <span aria-hidden="true" className="flex shrink-0 text-ink-faint">
          {icon}
        </span>
      ) : null}
      <span className="min-w-0 flex-1">{children}</span>
      {trailing ? (
        <span className="flex shrink-0 items-center gap-1 text-ink-faint">
          {trailing}
        </span>
      ) : null}
    </button>
  )
}
