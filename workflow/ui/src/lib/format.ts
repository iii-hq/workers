/** `950`, `1.5k`, `36k`, `1.2M`: a token count in a row's line. */
export function formatTokenCount(n: number): string {
  if (n < 1_000) return String(n)
  if (n < 1_000_000) {
    const k = n / 1_000
    return `${k >= 10 ? k.toFixed(0) : k.toFixed(1)}k`
  }
  const m = n / 1_000_000
  return `${m >= 10 ? m.toFixed(0) : m.toFixed(1)}M`
}
