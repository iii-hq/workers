import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import * as React from 'react'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'

export const Dialog = DialogPrimitive.Root
export const DialogTrigger = DialogPrimitive.Trigger
export const DialogClose = DialogPrimitive.Close

export const DialogContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(({ className, children, ...props }, ref) => (
  <DialogPrimitive.Portal>
    <PortalScope>
      <DialogPrimitive.Overlay className="iii-ui-motion-overlay fixed inset-0 z-50 bg-black/60" />
      <DialogPrimitive.Content
        ref={ref}
        className={cn(
          'iii-ui-motion-panel fixed top-1/2 left-1/2 z-50 -translate-1/2',
          'max-h-[85vh] w-full max-w-2xl overflow-y-auto',
          'rounded-xl bg-panel-raised p-6 font-sans text-ink shadow-floating',
          'focus-visible:outline-none',
          className,
        )}
        {...props}
      >
        {children}
        {/* Padded to a ~26px hit box (WCAG 2.2 SC 2.5.8 asks for 24) — the
            bare 14px glyph was the smallest target in the product. The
            right/top offsets compensate so the icon stays visually put. */}
        <DialogPrimitive.Close className="absolute right-2.5 top-2.5 rounded-sm p-1.5 text-ink-faint hover:text-ink hover:bg-surface-hover transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
          <X size={16} />
          <span className="sr-only">Close</span>
        </DialogPrimitive.Close>
      </DialogPrimitive.Content>
    </PortalScope>
  </DialogPrimitive.Portal>
))
DialogContent.displayName = 'DialogContent'

export const DialogTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn('font-sans text-[13px] font-semibold text-ink', className)}
    {...props}
  />
))
DialogTitle.displayName = 'DialogTitle'

export const DialogDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn('font-sans text-[12px] text-ink-faint', className)}
    {...props}
  />
))
DialogDescription.displayName = 'DialogDescription'
