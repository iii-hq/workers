import type { ReactNode, TextareaHTMLAttributes } from 'react'
import { Badge, uiClasses } from '@iii-dev/console-ui'
import { formatDuration } from '@iii-dev/console-ui/format'
import type { EvalStatus } from './types'

export function Field({
  label,
  hint,
  error,
  children,
  className = '',
}: {
  label?: string
  hint?: string
  error?: string
  children: ReactNode
  className?: string
}) {
  return (
    <label className={`${uiClasses.field} ${className}`}>
      {label ? <span className={uiClasses.fieldLabel}>{label}</span> : null}
      {children}
      {error ? (
        <span className={uiClasses.fieldError}>{error}</span>
      ) : hint ? (
        <span className={uiClasses.fieldDescription}>{hint}</span>
      ) : null}
    </label>
  )
}

export function TextArea({
  className = '',
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={`eval-ui-textarea ${className}`} {...props} />
}

export function StatusBadge({ status }: { status: EvalStatus }) {
  const variant =
    status === 'completed'
      ? 'ok'
      : status === 'failed'
        ? 'alert'
        : status === 'cancelled'
          ? 'warn'
          : 'default'
  return <Badge variant={variant}>{status}</Badge>
}

export function formatDate(timestamp?: number): string {
  if (!timestamp) return '—'
  return new Date(timestamp).toLocaleString()
}

export function formatMetric(
  value: number | undefined,
  kind: 'number' | 'percent' | 'duration' | 'cost' = 'number',
): string {
  if (value === undefined || !Number.isFinite(value)) return '—'
  if (kind === 'percent') return `${(value * 100).toFixed(1)}%`
  if (kind === 'duration') return formatDuration(value)
  if (kind === 'cost') return `$${value.toFixed(6)}`
  return new Intl.NumberFormat().format(value)
}

export function shortId(value: string): string {
  return value.length > 18 ? `${value.slice(0, 10)}…${value.slice(-6)}` : value
}
