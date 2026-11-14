/**
 * The console's first dialog API (Radix-shaped: `asChild` on the trigger
 * and close, a bare `DialogContent`), kept as a thin layer over the shadcn
 * (Base UI) dialog so worker UIs that import `Dialog` from
 * `@iii-dev/console-ui` get the same sheet, overlay, motion and close
 * affordance as the console. New console code reaches for
 * `components/shadcn/dialog` directly.
 */

import {
  type ComponentProps,
  isValidElement,
  type ReactElement,
  type ReactNode,
} from 'react'
import {
  Dialog as BaseDialog,
  DialogClose as BaseDialogClose,
  DialogContent as BaseDialogContent,
  DialogDescription as BaseDialogDescription,
  DialogTitle as BaseDialogTitle,
  DialogTrigger as BaseDialogTrigger,
} from '@/components/shadcn/dialog'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

export interface DialogProps
  extends Omit<ComponentProps<typeof BaseDialog>, 'onOpenChange'> {
  /** The first API's one-argument handler. */
  onOpenChange?: (open: boolean) => void
}

/**
 * The root. An Escape that a field inside already claimed (`SearchField`
 * clears its query on Escape and default-prevents the keydown) keeps the
 * dialog open, as it did under the first API; every other close reaches
 * `onOpenChange(false)`.
 */
export function Dialog({ onOpenChange, ...props }: DialogProps) {
  return (
    <BaseDialog
      {...props}
      onOpenChange={(open, details) => {
        const event = details.event
        if (
          !open &&
          event instanceof KeyboardEvent &&
          event.key === 'Escape' &&
          event.defaultPrevented
        ) {
          details.cancel()
          return
        }
        onOpenChange?.(open)
      }}
    />
  )
}

export interface DialogTriggerProps
  extends ComponentProps<typeof BaseDialogTrigger> {
  /** Radix: the single child element is the trigger (Base UI `render`). */
  asChild?: boolean
  children?: ReactNode
}

export function DialogTrigger({
  asChild,
  children,
  ...props
}: DialogTriggerProps) {
  if (asChild && isValidElement(children)) {
    return (
      <BaseDialogTrigger
        {...props}
        render={children as ReactElement<Record<string, unknown>>}
      />
    )
  }
  return <BaseDialogTrigger {...props}>{children}</BaseDialogTrigger>
}

export interface DialogCloseProps
  extends ComponentProps<typeof BaseDialogClose> {
  /** Radix: the single child element is the button (Base UI `render`). */
  asChild?: boolean
  children?: ReactNode
}

export function DialogClose({ asChild, children, ...props }: DialogCloseProps) {
  if (asChild && isValidElement(children)) {
    return (
      <BaseDialogClose
        {...props}
        render={children as ReactElement<Record<string, unknown>>}
      />
    )
  }
  return <BaseDialogClose {...props}>{children}</BaseDialogClose>
}

export type DialogContentProps = ComponentProps<typeof BaseDialogContent>

/**
 * Portalled, centered, with overlay and close affordance built in. The
 * legacy sheet is wider than shadcn's default (`max-w-2xl`) and scrolls
 * inside `85vh`; a caller narrows it with `className`.
 */
export function DialogContent({ className, ...props }: DialogContentProps) {
  return (
    <BaseDialogContent
      className={cn('max-h-[85vh] overflow-y-auto p-6 sm:max-w-2xl', className)}
      {...props}
    />
  )
}

export function DialogTitle({
  className,
  ...props
}: ComponentProps<typeof BaseDialogTitle>) {
  return (
    <BaseDialogTitle
      className={cn('text-[13px] font-semibold', className)}
      {...props}
    />
  )
}

export function DialogDescription({
  className,
  ...props
}: ComponentProps<typeof BaseDialogDescription>) {
  return (
    <BaseDialogDescription className={cn('text-xs', className)} {...props} />
  )
}
