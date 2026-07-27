import type { ReactNode, TextareaHTMLAttributes } from 'react'
import { Badge } from '@iii-dev/console-ui'
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
    <label className={`eval-ui-field ${className}`}>
      {label ? <span className="eval-ui-label">{label}</span> : null}
      {children}
      {error ? (
        <span className="eval-ui-field-error">{error}</span>
      ) : hint ? (
        <span className="eval-ui-hint">{hint}</span>
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
      ? 'accent'
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
  if (kind === 'duration') {
    return Math.abs(value) >= 1000
      ? `${(value / 1000).toFixed(2)}s`
      : `${value.toFixed(0)}ms`
  }
  if (kind === 'cost') return `$${value.toFixed(6)}`
  return new Intl.NumberFormat().format(value)
}

export function shortId(value: string): string {
  return value.length > 18 ? `${value.slice(0, 10)}…${value.slice(-6)}` : value
}
