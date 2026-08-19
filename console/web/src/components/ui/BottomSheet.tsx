import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import * as React from 'react'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'

export const BottomSheet = DialogPrimitive.Root
export const BottomSheetTrigger = DialogPrimitive.Trigger
export const BottomSheetClose = DialogPrimitive.Close

export const BottomSheetContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(({ className, children, ...props }, ref) => (
  <DialogPrimitive.Portal>
    <PortalScope>
      <DialogPrimitive.Overlay className="iii-ui-motion-overlay fixed inset-0 z-40 bg-black/55 sm:hidden" />
      <DialogPrimitive.Content
        ref={ref}
        className={cn(
          'iii-ui-motion-panel fixed inset-x-3 bottom-3 z-50 flex max-h-[calc(100dvh-1.5rem)] flex-col overflow-hidden',
          'rounded-lg border border-edge bg-panel-raised text-ink shadow-floating',
          'pb-[max(0.75rem,env(safe-area-inset-bottom))] focus:outline-none',
          className,
        )}
        {...props}
      >
        <div
          className="flex h-6 shrink-0 items-center justify-center"
          aria-hidden
        >
          <span className="h-1 w-10 rounded-full bg-ink-ghost/60" />
        </div>
        {children}
        <DialogPrimitive.Close className="absolute right-2 top-2 flex size-12 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus">
          <X className="size-5" aria-hidden />
          <span className="sr-only">Close</span>
        </DialogPrimitive.Close>
      </DialogPrimitive.Content>
    </PortalScope>
  </DialogPrimitive.Portal>
))
BottomSheetContent.displayName = 'BottomSheetContent'

export const BottomSheetTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn('font-sans text-lg font-semibold text-ink', className)}
    {...props}
  />
))
BottomSheetTitle.displayName = 'BottomSheetTitle'

export const BottomSheetDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn(
      'font-sans text-base leading-relaxed text-ink-faint',
      className,
    )}
    {...props}
  />
))
BottomSheetDescription.displayName = 'BottomSheetDescription'
