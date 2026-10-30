/**
 * The console's first tooltip API (Radix-shaped: `asChild`, `delayDuration`,
 * a `label` shorthand), kept as a thin layer over the shadcn (Base UI)
 * tooltip so worker UIs that import `Tooltip` from `@iii-dev/console-ui`
 * get the same pill, arrow, delay and motion as the console. New console
 * code reaches for `Hint` or `components/shadcn/tooltip` directly.
 */

import {
  type ComponentProps,
  createContext,
  isValidElement,
  type ReactElement,
  type ReactNode,
  useContext,
} from 'react'
import {
  Tooltip as BaseTooltip,
  TooltipContent as BaseTooltipContent,
  TooltipProvider as BaseTooltipProvider,
  TooltipTrigger as BaseTooltipTrigger,
} from '@/components/shadcn/tooltip'

export interface TooltipProviderProps
  extends ComponentProps<typeof BaseTooltipProvider> {
  /** Radix name for `delay`. */
  delayDuration?: number
  /** Radix name for `timeout`. */
  skipDelayDuration?: number
}

export function TooltipProvider({
  delayDuration,
  skipDelayDuration,
  delay,
  timeout,
  ...props
}: TooltipProviderProps) {
  return (
    <BaseTooltipProvider
      {...props}
      delay={delay ?? delayDuration}
      timeout={timeout ?? skipDelayDuration}
    />
  )
}

/** A root's `delayDuration`, read by its trigger (Base UI sets it there). */
const TriggerDelay = createContext<number | undefined>(undefined)

export interface TooltipProps
  extends Omit<ComponentProps<typeof BaseTooltip>, 'children'> {
  /** Radix name for the trigger's open delay, in milliseconds. */
  delayDuration?: number
  /** Shorthand: `children` become the trigger and this the content. */
  label?: ReactNode
  children?: ReactNode
}

/** The tooltip root; with `label`, also the trigger/content composition. */
export function Tooltip({
  label,
  delayDuration,
  children,
  ...props
}: TooltipProps) {
  if (label == null) {
    return (
      <TriggerDelay.Provider value={delayDuration}>
        <BaseTooltip {...props}>{children}</BaseTooltip>
      </TriggerDelay.Provider>
    )
  }
  return (
    <BaseTooltip {...props}>
      {isValidElement(children) ? (
        <BaseTooltipTrigger delay={delayDuration} render={children} />
      ) : (
        <BaseTooltipTrigger delay={delayDuration}>
          {children}
        </BaseTooltipTrigger>
      )}
      <TooltipContent>{label}</TooltipContent>
    </BaseTooltip>
  )
}

export interface TooltipTriggerProps
  extends ComponentProps<typeof BaseTooltipTrigger> {
  /** Radix: the single child element is the trigger (Base UI `render`). */
  asChild?: boolean
}

export function TooltipTrigger({
  asChild,
  children,
  ...props
}: TooltipTriggerProps) {
  const delay = useContext(TriggerDelay)
  if (asChild && isValidElement(children)) {
    return (
      <BaseTooltipTrigger
        delay={delay}
        {...props}
        render={children as ReactElement<Record<string, unknown>>}
      />
    )
  }
  return (
    <BaseTooltipTrigger delay={delay} {...props}>
      {children}
    </BaseTooltipTrigger>
  )
}

export interface TooltipContentProps
  extends ComponentProps<typeof BaseTooltipContent> {
  /** Radix flag; Base UI always keeps the pill inside the viewport. */
  avoidCollisions?: boolean
}

/** Bottom by default: most legacy triggers sit in top bars, where a
    top-side tooltip pops over the tab strip. Collisions still flip it. */
export function TooltipContent({
  side = 'bottom',
  sideOffset = 6,
  avoidCollisions: _avoidCollisions,
  ...props
}: TooltipContentProps) {
  return <BaseTooltipContent side={side} sideOffset={sideOffset} {...props} />
}
