import * as React from 'react'
import { Button } from './Button'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './Dialog'

export interface ConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: React.ReactNode
  /** Lines listed under the description, e.g. the unsaved items at stake. */
  details?: readonly string[]
  confirmLabel?: string
  cancelLabel?: string
  onConfirm: () => void
  onCancel?: () => void
}

/** The console's confirmation in place of `window.confirm`; cancel owns initial focus. */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  details,
  confirmLabel = 'Continue',
  cancelLabel = 'Cancel',
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const settle = (confirmed: boolean) => {
    onOpenChange(false)
    if (confirmed) onConfirm()
    else onCancel?.()
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) settle(false)
      }}
    >
      <DialogContent
        role="alertdialog"
        className="max-w-md"
        onOpenAutoFocus={(event) => {
          event.preventDefault()
          cancelRef.current?.focus()
        }}
      >
        <DialogTitle className="pr-8 text-[14px]">{title}</DialogTitle>
        {description ? (
          <DialogDescription className="mt-2 text-[13px] leading-relaxed">
            {description}
          </DialogDescription>
        ) : null}
        {details && details.length > 0 ? (
          <ul className="mt-3 space-y-1 rounded-sm bg-surface px-3 py-2 font-sans text-[12px] text-ink">
            {details.map((line) => (
              <li key={line} className="truncate">
                {line}
              </li>
            ))}
          </ul>
        ) : null}
        <div className="mt-5 flex justify-end gap-2">
          <Button
            ref={cancelRef}
            type="button"
            variant="pill"
            size="sm"
            onClick={() => settle(false)}
          >
            {cancelLabel}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={() => settle(true)}
          >
            {confirmLabel}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
