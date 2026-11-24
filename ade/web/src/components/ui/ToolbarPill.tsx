import type { ComponentProps } from 'react'
import { cn } from '@/lib/utils'

/**
 * The header's labelled pill: a card-filled, 32px control in the muted ink
 * that lifts to the hover surface and stays lifted while its popup is open.
 * Callers pass layout only (width, margin); the look lives here. Hand it to
 * a Base UI trigger through `render`, e.g.
 * `<PopoverTrigger render={<ToolbarPill className="max-w-56" />}>`.
 */
export function ToolbarPill({
  className,
  type = 'button',
  ...props
}: ComponentProps<'button'>) {
  return (
    <button
      type={type}
      data-slot="toolbar-pill"
      className={cn(
        'icon-host inline-flex h-8 shrink-0 items-center gap-2 rounded-field bg-card px-3 font-inter text-sm font-medium text-muted-foreground transition-colors duration-(--motion-duration-control) ease-(--motion-ease-standard) hover:bg-surface-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none data-open:bg-surface-hover data-open:text-foreground',
        className,
      )}
      {...props}
    />
  )
}
