import type { ComponentProps } from 'react'
import {
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { SelectTrigger } from '@/components/shadcn/select'
import { cn } from '@/lib/utils'

/*
 * Dialogs that lay out their own sections: the content drops the dialog's
 * own gap and padding, and the parts carry the looks those dialogs share.
 * Callers pass layout only (width, height, the max-width variable, flex).
 *
 * - `FormDialog*`: a form on the dialog surface with its hairline frame,
 *   titled at 15px (New project, Edit project, Conversation details).
 * - `PanelDialog*`: a panel that fills the dialog, titled at 16px, with a
 *   ruled header when it has one (Add folder).
 */

export function PanelDialogContent({
  className,
  ...props
}: ComponentProps<typeof DialogContent>) {
  return <DialogContent className={cn('gap-0 p-0', className)} {...props} />
}

export function FormDialogContent({
  className,
  ...props
}: ComponentProps<typeof DialogContent>) {
  return (
    <PanelDialogContent
      className={cn(
        'bg-dialog-surface text-foreground ring-1 ring-dialog-edge',
        className,
      )}
      {...props}
    />
  )
}

export function FormDialogTitle({
  className,
  ...props
}: ComponentProps<typeof DialogTitle>) {
  return (
    <DialogTitle
      className={cn('text-[15px] leading-5 font-semibold', className)}
      {...props}
    />
  )
}

export function FormDialogDescription({
  className,
  ...props
}: ComponentProps<typeof DialogDescription>) {
  return (
    <DialogDescription
      className={cn('text-body text-muted-foreground', className)}
      {...props}
    />
  )
}

/** A select in a form dialog: the card fill and hairline of its fields. */
export function FormSelectTrigger({
  className,
  ...props
}: ComponentProps<typeof SelectTrigger>) {
  return (
    <SelectTrigger
      className={cn('border-border bg-card', className)}
      {...props}
    />
  )
}

export function PanelDialogHeader({
  className,
  ...props
}: ComponentProps<typeof DialogHeader>) {
  return (
    <DialogHeader
      className={cn('gap-3 border-b border-border px-5 py-4 pr-14', className)}
      {...props}
    />
  )
}

export function PanelDialogTitle({
  className,
  ...props
}: ComponentProps<typeof DialogTitle>) {
  return (
    <DialogTitle
      className={cn('text-base font-medium', className)}
      {...props}
    />
  )
}

export function PanelDialogDescription({
  className,
  ...props
}: ComponentProps<typeof DialogDescription>) {
  return (
    <DialogDescription
      className={cn('text-sm text-muted-foreground', className)}
      {...props}
    />
  )
}
