/* One line of text asked for in a dialog: a branch name, a stash message,
   a commit author. Enter confirms, Escape cancels (the Dialog's own). */

import { Button, Dialog, DialogContent, DialogDescription, DialogTitle, Input } from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useId, useState } from 'react'

export interface TextDialogProps {
  open: boolean
  title: string
  description?: ReactNode
  label: string
  placeholder?: string
  initial?: string
  confirmLabel: string
  /** An empty value is allowed (it clears, or means "none"). */
  allowEmpty?: boolean
  /** Extra controls under the field, e.g. an "include unversioned files" box. */
  children?: ReactNode
  onConfirm: (value: string) => void
  onCancel: () => void
}

export function TextDialog({
  open,
  title,
  description,
  label,
  placeholder,
  initial = '',
  confirmLabel,
  allowEmpty = false,
  children,
  onConfirm,
  onCancel,
}: TextDialogProps) {
  const [value, setValue] = useState(initial)
  const fieldId = useId()
  // Each opening starts from the caller's value.
  useEffect(() => {
    if (open) setValue(initial)
  }, [open, initial])
  const ready = allowEmpty || value.trim() !== ''
  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : onCancel())}>
      <DialogContent className="shui-text-dialog">
        <DialogTitle>{title}</DialogTitle>
        {description ? <DialogDescription>{description}</DialogDescription> : null}
        <form
          className="shui-text-dialog-form"
          onSubmit={(event) => {
            event.preventDefault()
            if (ready) onConfirm(value.trim())
          }}
        >
          <label className="shui-text-dialog-label" htmlFor={fieldId}>
            {label}
          </label>
          <Input id={fieldId} value={value} onChange={setValue} placeholder={placeholder} autoFocus />
          {children}
          <div className="shui-text-dialog-actions">
            <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" size="sm" disabled={!ready}>
              {confirmLabel}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
