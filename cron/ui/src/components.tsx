import { uiClasses } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'

/** The console's field recipe: label above the control, hint or error below.
    Using it rather than local metrics is what keeps this page reading like
    the rest of the console. */
export function Field({
  label,
  hint,
  htmlFor,
  className,
  children,
}: {
  label: string
  hint?: ReactNode
  htmlFor?: string
  className?: string
  children: ReactNode
}) {
  const classes = className ? `${uiClasses.field} ${className}` : uiClasses.field
  return (
    <div className={classes}>
      <label className={uiClasses.fieldLabel} htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {hint ? <span className={uiClasses.fieldDescription}>{hint}</span> : null}
    </div>
  )
}

/** The library has no multi-line input, so this is the local one, painted to
    match the shared Input. */
export function TextArea({
  id,
  value,
  onChange,
  placeholder,
}: {
  id?: string
  value: string
  onChange: (next: string) => void
  placeholder?: string
}) {
  return (
    <textarea
      id={id}
      className="cron-ui-textarea"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      rows={3}
    />
  )
}
