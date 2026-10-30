'use client'

import { Tooltip as TooltipPrimitive } from '@base-ui/react/tooltip'
import { cn } from 'cn'
import {
  INVERSE_INK,
  INVERSE_SURFACE_VARS,
  NEUTRAL_SURFACE,
} from '@/lib/neutral-surface'
import { PortalScope } from '@/lib/ui-scope'

/**
 * The first tooltip waits `delay` so a pointer passing over a toolbar does
 * not flash one; once one is open, its neighbours open at once (Base UI's
 * provider group, for `timeout` after the last one closes) and skip the
 * entrance animation (`data-instant`). Mounted once at the app root.
 */
function TooltipProvider({
  delay = 400,
  closeDelay = 0,
  ...props
}: TooltipPrimitive.Provider.Props) {
  return (
    <TooltipPrimitive.Provider
      data-slot="tooltip-provider"
      delay={delay}
      closeDelay={closeDelay}
      {...props}
    />
  )
}

function Tooltip({ ...props }: TooltipPrimitive.Root.Props) {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />
}

function TooltipTrigger({ ...props }: TooltipPrimitive.Trigger.Props) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />
}

function TooltipContent({
  className,
  side = 'top',
  sideOffset = 4,
  align = 'center',
  alignOffset = 0,
  collisionPadding = 8,
  children,
  ...props
}: TooltipPrimitive.Popup.Props &
  Pick<
    TooltipPrimitive.Positioner.Props,
    'align' | 'alignOffset' | 'side' | 'sideOffset' | 'collisionPadding'
  >) {
  return (
    <TooltipPrimitive.Portal>
      {/* A worker's tooltip stays under its `[data-iii-ui]` CSS scope. */}
      <PortalScope>
        <TooltipPrimitive.Positioner
          align={align}
          alignOffset={alignOffset}
          side={side}
          sideOffset={sideOffset}
          collisionPadding={collisionPadding}
          className="isolate z-50"
        >
          <TooltipPrimitive.Popup
            data-slot="tooltip-content"
            className={cn(
              NEUTRAL_SURFACE,
              INVERSE_SURFACE_VARS,
              // shadcn base-nova: an inverse 12px pill with an arrow.
              'z-50 inline-flex w-fit max-w-[min(20rem,calc(100vw-1rem))] origin-(--transform-origin) items-center gap-1.5 rounded-md bg-foreground px-3 py-1.5 font-inter text-xs text-balance wrap-break-word text-background',
              // A key cap (Hint's `shortcut`, a worker's `Kbd`) sits flush
              // right in a translucent cap of the pill's own ink.
              'has-data-[slot=kbd]:pr-1.5 **:data-[slot=kbd]:relative **:data-[slot=kbd]:isolate **:data-[slot=kbd]:z-50 **:data-[slot=kbd]:inline-flex **:data-[slot=kbd]:h-4 **:data-[slot=kbd]:min-w-4 **:data-[slot=kbd]:items-center **:data-[slot=kbd]:justify-center **:data-[slot=kbd]:rounded-sm **:data-[slot=kbd]:bg-background/20 **:data-[slot=kbd]:px-1 **:data-[slot=kbd]:font-inter **:data-[slot=kbd]:text-[11px] **:data-[slot=kbd]:font-medium **:data-[slot=kbd]:tracking-wide **:data-[slot=kbd]:text-background',
              // Fade, zoom from 95% and a short slide away from the trigger
              // on the way in; a quicker fade out. A neighbour opened while
              // the group is warm appears at once, and reduced motion
              // drops the movement entirely.
              'ease-out data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-open:duration-150 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 data-closed:duration-100 data-[side=bottom]:slide-in-from-top-1 data-[side=inline-end]:slide-in-from-left-1 data-[side=inline-start]:slide-in-from-right-1 data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1 data-[side=top]:slide-in-from-bottom-1 data-instant:animate-none motion-reduce:animate-none',
              className,
            )}
            {...props}
          >
            {/* The pill is inverse; its content inherits inverted tokens, so
                copy styled for the page (`text-ink`, a worker's own CSS)
                reads on it in both themes. `contents` keeps the children in
                the pill's flex row. */}
            <span
              data-slot="tooltip-body"
              className={cn('contents', INVERSE_INK)}
            >
              {children}
            </span>
            <TooltipPrimitive.Arrow
              data-slot="tooltip-arrow"
              className="z-50 size-2.5 translate-y-[calc(-50%-2px)] rotate-45 rounded-[2px] bg-foreground fill-foreground data-[side=bottom]:top-1 data-[side=inline-end]:top-1/2! data-[side=inline-end]:-left-1 data-[side=inline-end]:-translate-y-1/2 data-[side=inline-start]:top-1/2! data-[side=inline-start]:-right-1 data-[side=inline-start]:-translate-y-1/2 data-[side=left]:top-1/2! data-[side=left]:-right-1 data-[side=left]:-translate-y-1/2 data-[side=right]:top-1/2! data-[side=right]:-left-1 data-[side=right]:-translate-y-1/2 data-[side=top]:-bottom-2.5"
            />
          </TooltipPrimitive.Popup>
        </TooltipPrimitive.Positioner>
      </PortalScope>
    </TooltipPrimitive.Portal>
  )
}

export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger }
