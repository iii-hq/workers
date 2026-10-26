/** Strings the rows and toasts show (the host formats nothing, design §5.5). */

import { IiiError } from '@iii-dev/native-ui'

const two = (n: number) => (n < 10 ? `0${n}` : String(n))

/** `14:03:27`, local time. */
export function clock(ms: number): string {
  const d = new Date(ms)
  if (Number.isNaN(d.getTime())) return ''
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`
}

/** `20261022-140327`, local time: a file name's stamp. */
export function stamp(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}${two(d.getMonth() + 1)}${two(d.getDate())}-${two(d.getHours())}${two(d.getMinutes())}${two(d.getSeconds())}`
}

/** `82 of 150 MB`, or `82 MB` when the total is unknown. */
export function megabytes(done: number, total?: number): string {
  const mb = (n: number) => String(Math.round(n / 1_000_000))
  return total ? `${mb(done)} of ${mb(total)} MB` : `${mb(done)} MB`
}

/** The person-facing reason of a failed call: the engine's words, never JSON. */
export function readable(err: unknown): string {
  if (err instanceof IiiError) {
    switch (err.code) {
      case 'timeout':
        return "The engine didn't answer in time"
      case 'not_connected':
      case 'connection_lost':
        return 'The engine is not connected'
      case 'function_not_found':
        return "The browser worker isn't running on the engine"
    }
  }
  const message = err instanceof Error ? err.message : typeof err === 'string' ? err : ''
  return clean(message) || 'Something went wrong'
}

/** A message that is itself JSON (`{"message": …}` or `{"error": …}`) gives its text. */
function clean(message: string): string {
  const text = message.trim()
  if (text.startsWith('{')) {
    try {
      const v = JSON.parse(text) as Record<string, unknown>
      const inner = v.message ?? v.error
      if (typeof inner === 'string') return clean(inner)
    } catch {
      // Not JSON after all: the text as it is.
    }
  }
  return text.replace(/^(Error|IiiError):\s*/, '')
}
