/**
 * Small pure formatting/encoding helpers shared across the chip, turn
 * summary and page: seconds and duration formatting, text truncation,
 * and PCM16→base64 encoding. Errors and bytes come from
 * `@iii-dev/console-ui/format`.
 */

export function formatSeconds(value: number | undefined): string {
  return typeof value === 'number' && Number.isFinite(value) ? value.toFixed(2) : '—'
}

export function formatDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return '0s'
  if (secs < 10) return `${secs.toFixed(1)}s`
  const total = Math.round(secs)
  if (total < 60) return `${total}s`
  const minutes = Math.floor(total / 60)
  return `${minutes}m ${total - minutes * 60}s`
}

/** A recording clock, `0:06` or `12:40`. */
export function formatClock(secs: number): string {
  const total = Number.isFinite(secs) && secs > 0 ? Math.floor(secs) : 0
  const minutes = Math.floor(total / 60)
  return `${minutes}:${String(total - minutes * 60).padStart(2, '0')}`
}

export function truncate(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text
  return `${text.slice(0, Math.max(0, maxChars - 1))}…`
}

/** Base64-encode a little-endian Int16 PCM buffer for the wire. */
export function base64FromInt16(samples: Int16Array): string {
  const bytes = new Uint8Array(samples.buffer, samples.byteOffset, samples.byteLength)
  let binary = ''
  const chunkSize = 0x8000
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    const chunk = bytes.subarray(offset, offset + chunkSize)
    binary += String.fromCharCode(...chunk)
  }
  return btoa(binary)
}
