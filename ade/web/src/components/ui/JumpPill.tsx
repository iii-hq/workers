import type { ComponentProps, ReactNode } from 'react'
import { buttonVariants } from '@/components/ui/Button'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * The frosted pill that floats over a scrolled list and jumps somewhere in
 * it ("latest", "action required", "1 question to answer"). 36px on touch
 * (with a 48px hit area), 32px with a fine pointer. `tone="warn"` paints the
 * words in the warning ink for something that blocks the agent. Callers
 * pass layout only; the look lives here.
 */
export function JumpPill({
  icon,
  tone = 'default',
  className,
  children,
  type = 'button',
  ...props
}: ComponentProps<'button'> & {
  icon: ReactNode
  tone?: 'default' | 'warn'
}) {
  return (
    <button
      type={type}
      data-slot="jump-pill"
      data-tone={tone}
      className={cn(
        buttonVariants({ variant: 'pill', size: 'sm' }),
        'iii-ui-motion-overlay relative h-9 shrink-0 rounded-full bg-panel-raised/80 pr-3 pl-2.5 font-medium text-base shadow-raised ring-1 ring-border backdrop-blur-md sm:h-8 sm:text-body',
        tone === 'warn' && 'text-warn',
        className,
      )}
      {...props}
    >
      <span
        className="absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
        aria-hidden="true"
      />
      {icon}
      <span>{children}</span>
    </button>
  )
}
