/**
 * A tooltip for an icon-only control, on the shadcn (Base UI) tooltip.
 *
 * `children` is the control itself; it keeps its own props and ref, the
 * tooltip only adds its hover and focus listeners.
 */

import type { ReactElement, ReactNode } from 'react'
import { Tooltip, TooltipContent, TooltipTrigger } from './shadcn/tooltip'

export interface HintProps {
  label: ReactNode
  side?: 'top' | 'bottom' | 'left' | 'right'
  align?: 'start' | 'center' | 'end'
  /** Nothing to say right now (a control with its own visible label). */
  disabled?: boolean
  /** On the pill, e.g. a wider `max-w-*` for a long message. */
  className?: string
  children: ReactElement
}

export function Hint({
  label,
  side = 'bottom',
  align = 'center',
  disabled = false,
  className,
  children,
}: HintProps) {
  if (disabled) return children
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side={side} align={align} className={className}>
        {label}
      </TooltipContent>
    </Tooltip>
  )
}
