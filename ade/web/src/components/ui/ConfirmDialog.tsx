import * as React from 'react'
import { Button } from '@/components/shadcn/button'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './Dialog'

// viewport: phone chrome — shared modal margins, stacked actions and touch targets.

export interface ConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The question, naming the thing at stake: `Delete index.html?`. */
  title: string
  /** One or two plain sentences: what happens, and whether it can be undone. */
  description?: React.ReactNode
  confirmLabel?: string
  cancelLabel?: string
  /** `danger` paints the confirm control in `alert` for destructive actions. */
  tone?: 'default' | 'danger'
  /**
   * The confirmed action. A promise keeps the dialog open while it runs
   * (the confirm control shows `busyLabel`, nothing else is clickable) and
   * closes it once it resolves; a rejection stays open with its message
   * under the description, and the confirm control tries again.
   */
  onConfirm: () => void | Promise<void>
  onCancel?: () => void
  /** The confirm control's label while an async `onConfirm` runs. */
  busyLabel?: string
  /** Why the action failed, from outside the dialog (it opens on the failure,
      the confirm control then tries again). A rejected `onConfirm` sets its
      own. While one shows, the controls read `Retry …` and `Close`. */
  error?: string | null
}

/**
 * The console's one alert dialog, in place of `window.confirm`: a title, a
 * short description, and the actions on their own band. Nothing else goes in
 * it. Cancel owns initial focus; Escape and the close control cancel.
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel = 'Continue',
  cancelLabel = 'Cancel',
  tone = 'default',
  onConfirm,
  onCancel,
  busyLabel,
  error: outsideError = null,
}: ConfirmDialogProps) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const [busy, setBusy] = React.useState(false)
  const [ownError, setError] = React.useState<string | null>(null)
  const error = ownError ?? outsideError
  const busyRef = React.useRef(false)
  // Each opening starts clean.
  React.useEffect(() => {
    if (!open) return
    setError(null)
  }, [open])
  const cancel = () => {
    if (busyRef.current) return
    onOpenChange(false)
    onCancel?.()
  }
  const confirm = () => {
    if (busyRef.current) return
    const result = onConfirm()
    if (!(result instanceof Promise)) {
      onOpenChange(false)
      return
    }
    busyRef.current = true
    setBusy(true)
    setError(null)
    result.then(
      () => {
        busyRef.current = false
        setBusy(false)
        onOpenChange(false)
      },
      (reason: unknown) => {
        busyRef.current = false
        setBusy(false)
        setError(reason instanceof Error ? reason.message : String(reason))
      },
    )
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) cancel()
      }}
    >
      <DialogContent
        role="alertdialog"
        aria-busy={busy || undefined}
        className="flex w-[calc(100%-24px)] max-w-[440px] flex-col overflow-hidden rounded-2xl p-0 font-inter"
        onOpenAutoFocus={(event) => {
          event.preventDefault()
          cancelRef.current?.focus()
        }}
      >
        {/* The text scrolls when it is taller than the dialog (at most 85vh), so
            the actions below always stay on screen. */}
        <div className="min-h-0 overflow-y-auto px-6 pt-6 pb-5">
          <DialogTitle className="pr-8 font-inter text-[16px] leading-6 font-semibold break-words">
            {title}
          </DialogTitle>
          {description ? (
            <DialogDescription className="mt-1.5 font-inter text-[14px] leading-[22px] break-words">
              {description}
            </DialogDescription>
          ) : null}
          {error ? (
            <p
              role="alert"
              className="mt-3 font-inter text-[14px] leading-[22px] break-words text-alert"
            >
              {error}
            </p>
          ) : null}
        </div>
        {/* The actions sit on their own band, one surface step down. */}
        <div className="flex shrink-0 flex-col-reverse justify-end gap-2 bg-surface px-6 py-4 sm:flex-row">
          <Button
            ref={cancelRef}
            type="button"
            variant="secondary"
            size="lg"
            disabled={busy}
            className="h-11 sm:h-9"
            onClick={cancel}
          >
            {error ? 'Close' : cancelLabel}
          </Button>
          <Button
            type="button"
            variant={tone === 'danger' ? 'destructive' : 'default'}
            size="lg"
            disabled={busy}
            className="h-11 sm:h-9"
            onClick={confirm}
          >
            {busy && busyLabel
              ? busyLabel
              : error
                ? `Retry ${confirmLabel.charAt(0).toLowerCase()}${confirmLabel.slice(1)}`
                : confirmLabel}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

export interface ConfirmOptions
  extends Pick<
    ConfirmDialogProps,
    'title' | 'description' | 'confirmLabel' | 'cancelLabel' | 'tone'
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
      confirmLabel={options?.confirmLabel}
      cancelLabel={options?.cancelLabel}
      tone={options?.tone}
      onConfirm={() => resolve(true)}
      onCancel={() => resolve(false)}
    />
  )
  return { confirm, dialog }
}
