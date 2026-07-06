import { useCallback, useRef, useState } from 'react'
import type { VisualizationSpan } from './traceTransform'

export type SpanType = 'trigger' | 'enqueue' | 'function'

export function classifySpanType(span: VisualizationSpan): SpanType {
  const attrs = span.attributes || {}
  if (
    attrs['messaging.operation.type'] === 'publish' ||
    attrs['messaging.destination.name']
  )
    return 'enqueue'
  if (
    span.name.startsWith('trigger') ||
    attrs['iii.function.kind'] !== undefined
  )
    return 'trigger'
  return 'function'
}

export function formatTimestamp(timestampMs: number): string {
  const date = new Date(timestampMs)
  return date.toLocaleTimeString('en-US', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    fractionalSecondDigits: 3,
  })
}

export function formatRelative(offsetMs: number): string {
  if (offsetMs < 0) return `-${formatRelative(-offsetMs)}`
  if (offsetMs < 1) return `+${(offsetMs * 1000).toFixed(0)}μs`
  if (offsetMs < 1000) return `+${offsetMs.toFixed(1)}ms`
  return `+${(offsetMs / 1000).toFixed(2)}s`
}

export function formatDuration(ms: number): string {
  if (ms < 0.001) return '0μs'
  if (ms < 1) return `${(ms * 1000).toFixed(0)}μs`
  if (ms < 1000) return `${ms.toFixed(2)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

export function useCopyToClipboard(timeout = 2000) {
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const copy = useCallback(
    (key: string, text: string) => {
      navigator.clipboard.writeText(text).catch(() => {})
      setCopiedKey(key)
      if (timeoutRef.current) clearTimeout(timeoutRef.current)
      timeoutRef.current = setTimeout(() => setCopiedKey(null), timeout)
    },
    [timeout],
  )

  return { copiedKey, copy }
}

export function getServiceName(span: {
  service_name?: string
  name: string
}): string {
  return span.service_name || span.name.split('.')[0]
}

// Status pill colors flow from the design tokens (`--success`, `--error`,
// `--muted`) so SpanPanel re-themes correctly under `[data-theme="light"]`
// or in any host that overrides the CSS custom properties. Subtle bg/border
// shades use `color-mix(in srgb, var(--token) 8%, transparent)` for an
// 8%-opacity tint without baking in an rgba literal.
export const STATUS_CONFIG: Record<
  string,
  { color: string; bg: string; border: string; label: string }
> = {
  ok: {
    color: 'var(--success)',
    bg: 'color-mix(in srgb, var(--success) 8%, transparent)',
    border: 'color-mix(in srgb, var(--success) 15%, transparent)',
    label: 'OK',
  },
  error: {
    color: 'var(--error)',
    bg: 'color-mix(in srgb, var(--error) 8%, transparent)',
    border: 'color-mix(in srgb, var(--error) 15%, transparent)',
    label: 'ERROR',
  },
  unset: {
    color: 'var(--muted)',
    bg: 'color-mix(in srgb, var(--muted) 8%, transparent)',
    border: 'color-mix(in srgb, var(--muted) 15%, transparent)',
    label: 'UNSET',
  },
  default: {
    color: 'var(--muted)',
    bg: 'color-mix(in srgb, var(--muted) 8%, transparent)',
    border: 'color-mix(in srgb, var(--muted) 15%, transparent)',
    label: 'UNKNOWN',
  },
}
