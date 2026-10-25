import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import * as React from 'react'
import {
  INVERSE_INK,
  INVERSE_SURFACE_VARS,
  NEUTRAL_SURFACE,
} from '@/lib/neutral-surface'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'

export const TooltipProvider = TooltipPrimitive.Provider
export const TooltipTrigger = TooltipPrimitive.Trigger

export interface TooltipProps extends TooltipPrimitive.TooltipProps {
  /** Shorthand: `children` become the trigger (`asChild`) and this the content. */
  label?: React.ReactNode
}

/** Radix root; with `label`, also the trigger/content composition. */
export function Tooltip({ label, children, ...props }: TooltipProps) {
  if (label == null) {
    return <TooltipPrimitive.Root {...props}>{children}</TooltipPrimitive.Root>
  }
  return (
    <TooltipPrimitive.Root {...props}>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </TooltipPrimitive.Root>
  )
}

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
      children,
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
            // The same recipe as the shadcn tooltip (components/shadcn/tooltip):
            // ink on a foreground pill, 12px, so every tooltip in the console
            // reads the same whichever primitive put it there.
            NEUTRAL_SURFACE,
            INVERSE_SURFACE_VARS,
            'iii-ui-motion-overlay z-50 max-w-[min(24rem,calc(100vw-1rem))] rounded-md bg-foreground px-3 py-1.5 font-inter text-xs wrap-break-word text-background',
            className,
          )}
          {...props}
        >
          {/* The pill is inverse; its content inherits inverted tokens, so
              a worker's help copy written in `--color-ink` reads on it. */}
          <span
            data-slot="tooltip-body"
            className={cn('contents', INVERSE_INK)}
          >
            {children}
          </span>
        </TooltipPrimitive.Content>
      </PortalScope>
    </TooltipPrimitive.Portal>
  ),
)
TooltipContent.displayName = 'TooltipContent'
