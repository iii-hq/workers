// Pieces the review dialogs share: the Dialog (a BottomSheet on a phone), a
// labelled field with its hint, help and status line, and the action row.
import {
  BottomSheet,
  BottomSheetContent,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  uiClasses,
} from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useState } from 'react'

// The shared BottomSheet is hidden from the `md` breakpoint (768 px) up, so a
// narrow pane in a wide window gets the Dialog; only a phone gets the sheet.
const PHONE_QUERY = '(max-width: 767px)'

export function usePhoneViewport(): boolean {
  const [phone, setPhone] = useState(
    () => typeof window !== 'undefined' && window.matchMedia?.(PHONE_QUERY).matches === true,
  )
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const media = window.matchMedia(PHONE_QUERY)
    const update = () => setPhone(media.matches)
    update()
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [])
  return phone
}

/** A dialog on a pane or a desktop, a bottom sheet on a phone; `children` learns which. */
export function DialogFrame({
  open,
  onOpenChange,
  title,
  description,
  narrow,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  narrow: boolean
  children: (layout: { sheet: boolean; narrow: boolean }) => ReactNode
}) {
  const phone = usePhoneViewport()
  const sheet = narrow && phone
  if (sheet) {
    return (
      <BottomSheet open={open} onOpenChange={onOpenChange}>
        <BottomSheetContent className="eval-ui-val-sheet" heading={title} description={description}>
          {children({ sheet, narrow })}
        </BottomSheetContent>
      </BottomSheet>
    )
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="eval-ui-val-dialog">
        <div className="eval-ui-val-header">
          <DialogTitle className="eval-ui-val-title">{title}</DialogTitle>
          {description ? <DialogDescription className="eval-ui-val-desc">{description}</DialogDescription> : null}
        </div>
        {children({ sheet, narrow })}
      </DialogContent>
    </Dialog>
  )
}

/** Cancel and the main action: side by side, or stacked with the main one first on a phone sheet. */
export function FormActions({
  sheet,
  narrow,
  cancel,
  submit,
  note,
}: {
  sheet: boolean
  narrow: boolean
  cancel: ReactNode
  submit: ReactNode
  /** A line at the start of the row, above the stacked actions on a phone sheet (`Nothing starts until you press Start.`). */
  note?: ReactNode
}) {
  return (
    <div className="eval-ui-val-actions" data-stacked={narrow || undefined} data-sheet={sheet || undefined}>
      {note ? <span className="eval-ui-rv-actions-note">{note}</span> : null}
      {sheet ? (
        <>
          {submit}
          {cancel}
        </>
      ) : (
        <>
          {cancel}
          {submit}
        </>
      )}
    </div>
  )
}

/** One status line under a field: found, a message, an error. */
export function FieldLine({
  tone,
  icon,
  role,
  id,
  children,
}: {
  tone?: 'ok' | 'alert' | 'warn'
  icon?: ReactNode
  role?: 'status' | 'alert'
  id?: string
  children: ReactNode
}) {
  return (
    <span id={id} className="eval-ui-val-line" data-tone={tone} role={role}>
      {icon}
      <span>{children}</span>
    </span>
  )
}

/**
 * A labelled field: the label, a quiet hint beside it, the control, help under
 * it, then a status line. `id` names the control the label is for; a group of
 * controls (a segmented choice) has none and names itself.
 */
export function FormField({
  id,
  label,
  hint,
  help,
  line,
  children,
}: {
  id?: string
  label: string
  hint?: string
  help?: ReactNode
  line?: ReactNode
  children: ReactNode
}) {
  return (
    <div className={uiClasses.field}>
      <div className="eval-ui-val-fieldhead">
        {id ? (
          <label className={uiClasses.fieldLabel} htmlFor={id}>
            {label}
          </label>
        ) : (
          <span className={uiClasses.fieldLabel}>{label}</span>
        )}
        {hint ? <span className="eval-ui-val-quiet eval-ui-val-small">{hint}</span> : null}
      </div>
      {children}
      {help ? <p className="eval-ui-val-note">{help}</p> : null}
      {line}
    </div>
  )
}

/** The console has no multi-line input: a native one that looks like `Input`. */
export function TextArea({
  id,
  value,
  onChange,
  placeholder,
  invalid,
  disabled,
  rows = 3,
  maxLength,
  describedBy,
}: {
  id: string
  value: string
  onChange: (next: string) => void
  placeholder?: string
  invalid?: boolean
  disabled?: boolean
  rows?: number
  maxLength?: number
  describedBy?: string
}) {
  return (
    <textarea
      id={id}
      className="eval-ui-rv-textarea"
      value={value}
      rows={rows}
      maxLength={maxLength}
      placeholder={placeholder}
      disabled={disabled}
      aria-invalid={invalid || undefined}
      aria-describedby={describedBy}
      onChange={(event) => onChange(event.target.value)}
    />
  )
}
