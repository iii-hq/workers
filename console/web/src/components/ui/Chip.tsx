import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

export type ChipTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'

export interface ChipProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: ChipTone
  selected?: boolean
}

export const Chip = React.forwardRef<HTMLSpanElement, ChipProps>(
  ({ className, tone = 'neutral', selected, ...props }, ref) => (
    <span
      ref={ref}
      data-tone={tone === 'neutral' ? undefined : tone}
      data-selected={selected || undefined}
      className={cn(uiClasses.chip, className)}
      {...props}
    />
  ),
)
Chip.displayName = 'Chip'
