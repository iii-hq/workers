/**
 * One readable string out of whatever a rejected `host.iii.trigger` throws.
 *
 * The browser SDK rejects with the wire error *object*, not an `Error`, so
 * `err instanceof Error ? err.message : String(err)` renders `[object Object]`.
 */

const TRANSPORT_CODES = new Set(['invocation_failed', 'handler_error'])

/**
 * @typedef {{ code?: string, inner_code?: string, message?: string }} ErrorBody
 */

/**
 * @param {unknown} value
 * @returns {ErrorBody | null}
 */
function bodyOf(value) {
  if (typeof value !== 'object' || value === null) return null
  const { code, inner_code, message } = /** @type {Record<string, unknown>} */ (value)
  return {
    code: typeof code === 'string' ? code : undefined,
    inner_code: typeof inner_code === 'string' ? inner_code : undefined,
    message: typeof message === 'string' ? message : undefined,
  }
}

/**
 * Peel `handler error: {json}` wrappers until the message stops being one.
 *
 * @param {ErrorBody} body
 * @returns {ErrorBody}
 */
function unwrap(body) {
  let current = body
  for (let depth = 0; depth < 4; depth++) {
    const text = current.message
    if (!text) return current
    const brace = text.indexOf('{')
    if (brace === -1) return current
    /** @type {unknown} */
    let parsed
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

/**
 * @param {unknown} err
 * @returns {string}
 */
export function errText(err) {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err

  const body = bodyOf(err)
  if (body?.message) {
    const { code, inner_code, message: unwrapped } = unwrap(body)
    const message = unwrapped?.replace(/^handler error: /, '')
    const label = code && !TRANSPORT_CODES.has(code) ? code : null
    if (!label) return message ?? String(err)
    return `${label}${inner_code ? ` (${inner_code})` : ''}: ${message}`
  }

  if (typeof err === 'object' && err !== null) {
    try {
      return JSON.stringify(err) ?? 'unknown error'
    } catch {
      return 'unknown error'
    }
  }
  return String(err)
}
