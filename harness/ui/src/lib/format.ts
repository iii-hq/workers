/** Token-count formatting shared by the chip, popover, and metrics card. */

export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '0'
  if (n >= 1_000_000) return `${trimmed(n / 1_000_000)}m`
  if (n >= 1000) return `${trimmed(n / 1000)}k`
  return String(Math.round(n))
}

export function formatCost(usd: number): string {
  return `$${usd.toFixed(4)}`
}

function trimmed(value: number): string {
  const fixed = value.toFixed(1)
  return fixed.endsWith('.0') ? fixed.slice(0, -2) : fixed
}
