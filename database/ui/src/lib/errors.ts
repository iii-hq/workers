/**
 * One readable string out of whatever a rejected `host.iii.trigger` throws.
 *
 * The browser SDK rejects with the wire error *object*, not an `Error`, so the
 * usual `err instanceof Error ? err.message : String(err)` rendered every
 * failed read as `[object Object]` — lowercased by the page's own CSS into
 * `[object object]`. The whole point of a SQL panel is to iterate on errors.
 *
 * The envelope nests: the transport says `invocation_failed` and hides the
 * useful part inside `message` as `handler error: {"code":…,"message":…}`.
 * Unwrap to the innermost message and prefix the handler's own code, so a
 * missing table reads `DRIVER_ERROR (1146): Table 'x' doesn't exist` rather
 * than the transport's generic failure.
 */

/** Codes that only say "the call failed" — never worth showing. */
const TRANSPORT_CODES = new Set(['invocation_failed', 'handler_error'])

interface ErrorBody {
  code?: string
  inner_code?: string
  message?: string
}

function bodyOf(value: unknown): ErrorBody | null {
  if (typeof value !== 'object' || value === null) return null
  const { code, inner_code, message } = value as Record<string, unknown>
  return {
    code: typeof code === 'string' ? code : undefined,
    inner_code: typeof inner_code === 'string' ? inner_code : undefined,
    message: typeof message === 'string' ? message : undefined,
  }
}

/**
 * Peel `handler error: {json}` wrappers until the message stops being one.
 * Bounded, because a malformed payload must not spin.
 */
function unwrap(body: ErrorBody): ErrorBody {
  let current = body
  for (let depth = 0; depth < 4; depth++) {
    const text = current.message
    if (!text) return current
    const brace = text.indexOf('{')
    if (brace === -1) return current
    let parsed: unknown
    try {
      parsed = JSON.parse(text.slice(brace))
    } catch {
      return current
    }
    const inner = bodyOf(parsed)
    if (!inner?.message) return current
    current = inner
  }
  return current
}

export function errText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err

  const body = bodyOf(err)
  if (body?.message) {
    const { code, inner_code, message } = unwrap(body)
    const label = code && !TRANSPORT_CODES.has(code) ? code : null
    if (!label) return message ?? String(err)
    return `${label}${inner_code ? ` (${inner_code})` : ''}: ${message}`
  }

  // Last resort: something unrecognised. Anything beats `[object Object]`,
  // which is what `String()` gives for every object — the bug this exists to
  // stop. Only non-objects are safe to stringify that way.
  if (typeof err === 'object' && err !== null) {
    try {
      return JSON.stringify(err) ?? 'unknown error'
    } catch {
      return 'unknown error'
    }
  }
  return String(err)
}
