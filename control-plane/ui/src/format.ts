import { formatRelative } from '@iii-dev/console-ui/format'

export function formatAge(input: Parameters<typeof formatRelative>[0]): string {
  const relative = formatRelative(input)
  return relative && relative !== 'just now' ? `${relative} ago` : relative
}

export function formatTokenCount(n: number): string {
  if (n < 1_000) return String(n)
  if (n < 1_000_000) {
    const k = n / 1_000
    return `${k >= 10 ? k.toFixed(0) : k.toFixed(1)}k`
  }
  const m = n / 1_000_000
  return `${m >= 10 ? m.toFixed(0) : m.toFixed(1)}M`
}
