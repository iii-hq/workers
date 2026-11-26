import type { ComponentProps } from 'react'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the pill is 40px on touch and 32px on desktop,
// like the model pill beside it; see viewport-breakpoint-conformance.test.ts.

/**
 * The composer toolbar's quiet pill: icon, label and chevron in the muted
 * ink, a background only on hover or while its popup is open, like the model
 * and agent pickers beside it. `Select`'s `pill` appearance renders the same
 * look. Callers pass layout only (width, margin); the look lives here. Hand
 * it to a Base UI trigger through `render`, e.g.
 * `<PopoverTrigger render={<ComposerPill />}>`.
 */
export const composerPillClass =
  'inline-flex h-10 max-w-full min-w-0 items-center gap-x-1.5 rounded-lg px-2 font-inter text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 data-open:bg-surface-hover data-open:text-ink data-[popup-open]:bg-surface-hover data-[popup-open]:text-ink data-[state=open]:bg-surface-hover data-[state=open]:text-ink sm:h-8 sm:text-sm'

export function ComposerPill({
  className,
  type = 'button',
  ...props
}: ComponentProps<'button'>) {
  return (
    <button
      type={type}
      data-slot="composer-pill"
      className={cn(composerPillClass, className)}
      {...props}
    />
  )
}
