import { ShadcnButton } from '@iii-dev/console-ui'
import { CircleAlert, type LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'

/** A one-line alert above the content: what went wrong, why, and one way out. */
export function Notice({
  title,
  detail,
  actionLabel,
  onAction,
}: {
  title: string
  detail?: string
  actionLabel?: string
  onAction?: () => void
}) {
  return (
    <div className="kb-notice" role="alert">
      <CircleAlert size={16} />
      <div className="kb-notice__text">
        <p className="kb-notice__title">{title}</p>
        {detail ? <p className="kb-notice__detail">{detail}</p> : null}
      </div>
      {actionLabel && onAction ? (
        <ShadcnButton onClick={onAction} size="sm" type="button" variant="outline">
          {actionLabel}
        </ShadcnButton>
      ) : null}
    </div>
  )
}

/** A centred empty state: icon tile, title, one sentence, an optional action. */
export function Empty({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon: LucideIcon
  title: string
  description?: string
  action?: ReactNode
}) {
  return (
    <div className="kb-empty">
      <span aria-hidden="true" className="kb-empty__icon">
        <Icon size={16} />
      </span>
      <p className="kb-empty__title">{title}</p>
      {description ? <p className="kb-empty__description">{description}</p> : null}
      {action ? <div className="kb-empty__action">{action}</div> : null}
    </div>
  )
}

/** A form field: a 12px label over the control, then a hint or an error. */
export function Field({
  label,
  htmlFor,
  hint,
  error,
  children,
}: {
  label: string
  htmlFor?: string
  hint?: ReactNode
  error?: string | null
  children: ReactNode
}) {
  return (
    <div className="kb-field">
      <label className="kb-field__label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {error ? (
        <p className="kb-field__error" role="alert">
          {error}
        </p>
      ) : hint ? (
        <p className="kb-field__hint">{hint}</p>
      ) : null}
    </div>
  )
}
