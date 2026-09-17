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
  /** `danger` paints the confirm control in `alert` for destructive actions. */
  tone?: 'default' | 'danger'
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
  tone = 'default',
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
            {details.map((line, index) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional and may repeat
                key={`${index}:${line}`}
                className="truncate"
              >
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
            className={
              tone === 'danger'
                ? 'bg-alert text-white hover:bg-alert/90'
                : undefined
            }
            onClick={() => settle(true)}
          >
            {confirmLabel}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

export interface ConfirmOptions
  extends Pick<
    ConfirmDialogProps,
    | 'title'
    | 'description'
    | 'details'
    | 'confirmLabel'
    | 'cancelLabel'
    | 'tone'
  > {}

/**
 * `window.confirm` shaped around `ConfirmDialog`: render `dialog` once in the
 * component, then `await confirm({ title, … })` where the native box used to
 * be. Escape, the close control and Cancel resolve `false`; a second call
 * while one is open cancels the first; unmounting cancels whatever is open.
 */
export function useConfirm(): {
  confirm: (options: ConfirmOptions) => Promise<boolean>
  dialog: React.ReactNode
} {
  const [options, setOptions] = React.useState<ConfirmOptions | null>(null)
  const resolveRef = React.useRef<((confirmed: boolean) => void) | null>(null)
  const resolve = (confirmed: boolean) => {
    resolveRef.current?.(confirmed)
    resolveRef.current = null
  }
  const confirm = React.useCallback(
    (next: ConfirmOptions) =>
      new Promise<boolean>((resolvePromise) => {
        resolveRef.current?.(false)
        resolveRef.current = resolvePromise
        setOptions(next)
      }),
    [],
  )
  React.useEffect(() => () => resolveRef.current?.(false), [])
  // ConfirmDialog closes (onOpenChange) before it reports the outcome, so
  // only onConfirm/onCancel settle the promise.
  const dialog = (
    <ConfirmDialog
      open={options !== null}
      onOpenChange={(open) => {
        if (!open) setOptions(null)
      }}
      title={options?.title ?? ''}
      description={options?.description}
      details={options?.details}
      confirmLabel={options?.confirmLabel}
      cancelLabel={options?.cancelLabel}
      tone={options?.tone}
      onConfirm={() => resolve(true)}
      onCancel={() => resolve(false)}
    />
  )
  return { confirm, dialog }
}
