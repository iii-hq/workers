import * as React from 'react'
import { Badge } from '@/components/shadcn/badge'
import { cn } from '@/lib/utils'

export type ChipTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'

export interface ChipProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: ChipTone
  selected?: boolean
}

const TONE: Record<ChipTone, string> = {
  neutral: 'bg-muted text-muted-foreground',
  accent: 'bg-accent-muted text-accent',
  success: 'bg-ok-muted text-ok',
  warning: 'bg-warn-muted text-warn',
  danger: 'bg-alert-muted text-alert',
}

/**
 * A small label with a tone: the shadcn badge at the console's 24px chip
 * height. `selected` draws the `border` edge on the selected wash.
 */
export const Chip = React.forwardRef<HTMLSpanElement, ChipProps>(
  ({ className, tone = 'neutral', selected, ...props }, ref) => (
    <Badge
      ref={ref}
      variant="outline"
      data-tone={tone === 'neutral' ? undefined : tone}
      data-selected={selected || undefined}
      className={cn(
        'h-6 max-w-full gap-1.5 rounded-md border-transparent px-2 py-0.5 font-sans text-xs font-medium',
        TONE[tone],
        selected && 'border-border bg-surface-selected text-foreground',
        className,
      )}
      {...props}
    />
  ),
)
Chip.displayName = 'Chip'
