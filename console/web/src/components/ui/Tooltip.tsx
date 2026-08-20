import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import * as React from 'react'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'

export const TooltipProvider = TooltipPrimitive.Provider
export const Tooltip = TooltipPrimitive.Root
export const TooltipTrigger = TooltipPrimitive.Trigger

export const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(
  (
    {
      className,
      side = 'bottom',
      sideOffset = 6,
      collisionPadding = 8,
      ...props
    },
    ref,
  ) => (
    <TooltipPrimitive.Portal>
      <PortalScope>
        {/* Bottom by default: most triggers sit in top bars, where a top-side
          tooltip pops over the tab strip. Radix collision-flips at viewport
          edges, and `side=` remains available for deliberate overrides. */}
        <TooltipPrimitive.Content
          ref={ref}
          side={side}
          sideOffset={sideOffset}
          collisionPadding={collisionPadding}
          className={cn(
            'iii-ui-motion-overlay z-50 max-w-[min(24rem,calc(100vw-1rem))] rounded-sm bg-panel-raised px-2.5 py-1.5 font-sans text-[12px] wrap-break-word text-ink shadow-raised',
            className,
          )}
          {...props}
        />
      </PortalScope>
    </TooltipPrimitive.Portal>
  ),
)
TooltipContent.displayName = 'TooltipContent'
