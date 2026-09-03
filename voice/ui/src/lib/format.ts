/**
 * Small pure formatting/encoding helpers shared across the chip, turn
 * summary and page: error-to-string, seconds formatting, text truncation,
 * markdown code-fence stripping, and PCM16→base64 encoding.
 */

export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return 'unknown error'
}

export function formatSeconds(value: number | undefined): string {
  return typeof value === 'number' && Number.isFinite(value) ? value.toFixed(2) : '—'
}

export function truncate(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text
  return `${text.slice(0, Math.max(0, maxChars - 1))}…`
}

/** Replace fenced code blocks with a spoken-friendly placeholder before
    handing text to text-to-speech. */
export function stripCodeFences(text: string): string {
  return text.replace(/```[\s\S]*?```/g, 'code block')
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
