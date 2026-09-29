import { formatRelative } from '@iii-dev/console-ui/format'

/** `5m ago` / `just now`; empty for unparsable input. */
export function ago(value: string): string {
  const rel = formatRelative(value)
  return rel && rel !== 'just now' ? `${rel} ago` : rel
}

export function formatCount(n: number): string {
  if (n < 1000) return `${n}`
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`
  return `${(n / 1_000_000).toFixed(1)}M`
}
