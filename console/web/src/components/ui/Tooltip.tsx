import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import * as React from 'react'
import { cn } from '@/lib/utils'

export const TooltipProvider = TooltipPrimitive.Provider
export const Tooltip = TooltipPrimitive.Root
export const TooltipTrigger = TooltipPrimitive.Trigger

export const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, side = 'bottom', sideOffset = 6, ...props }, ref) => (
  <TooltipPrimitive.Portal>
    {/* Bottom by default: most triggers sit in top bars, where a top-side
        tooltip pops over the tab strip. Radix still collision-flips near
        the viewport's bottom edge, and `side=` overrides per call site. */}
    <TooltipPrimitive.Content
      ref={ref}
      side={side}
      sideOffset={sideOffset}
      className={cn(
        'z-50 rounded-sm bg-panel-raised px-2.5 py-1.5 font-mono text-[12px] text-ink lowercase shadow-raised',
        className,
      )}
      {...props}
    />
  </TooltipPrimitive.Portal>
))
TooltipContent.displayName = 'TooltipContent'
