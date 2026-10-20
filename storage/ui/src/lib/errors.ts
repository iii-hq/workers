/**
 * What a rejected `host.iii.trigger` means for this UI, beyond its text.
 *
 * The browser SDK rejects with the wire error object, and the worker's own
 * error (`{"code":"BUCKET_UNAVAILABLE",…}`) usually sits inside the
 * transport's `handler error: {json}` message. `errorMessage` and
 * `errorCode` (console-ui/format) give the text and the code; this adds the
 * worker error's other fields and the "function not registered" check.
 */

import { errorCode, errorMessage } from '@iii-dev/console-ui/format'

export { errorCode, errorMessage }

type Body = Record<string, unknown>

function record(value: unknown): Body | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Body) : null
}

/**
 * The innermost error object a rejection carries: the worker's
 * `{code, …fields}` once the `handler error: {json}` wrappers are peeled.
 * Bounded, so a malformed payload cannot spin.
 */
export function innerError(err: unknown): Body | null {
  let current = record(err)
  if (!current) return null
  for (let depth = 0; depth < 4; depth++) {
    const nested = record(current.error)
    const text = typeof current.message === 'string' ? current.message : null
    if (!text && nested) {
      current = nested
      continue
    }
    const brace = text?.indexOf('{') ?? -1
    if (!text || brace === -1) return current
    let parsed: Body | null = null
    try {
      parsed = record(JSON.parse(text.slice(brace)))
    } catch {
      return current
    }
    if (!parsed || (typeof parsed.code !== 'string' && typeof parsed.message !== 'string')) return current
    current = parsed
  }
  return current
}

/**
 * A call to a function the engine does not have: the worker is not running,
 * or it predates the function. Never a worker's own `*_NOT_FOUND` code.
 */
export function isMissingFunction(err: unknown): boolean {
  const codes = [errorCode(err), record(err)?.code].filter((c): c is string => typeof c === 'string')
  if (codes.some((code) => code.toLowerCase() === 'function_not_found')) return true
  if (codes.some((code) => /^[A-Z][A-Z0-9_]+$/.test(code))) return false
  return /function[_ ]not[_ ]found|no such function|function \S+ (?:is )?not (?:found|registered)/i.test(
    errorMessage(err),
  )
}

/** `BUCKET_UNAVAILABLE`: the bucket is configured but its backend could not be built. */
export interface BucketUnavailable {
  bucket?: string
  stage?: string
  message: string
}

export function bucketUnavailable(err: unknown): BucketUnavailable | null {
  const inner = innerError(err)
  if (!inner || inner.code !== 'BUCKET_UNAVAILABLE') return null
  return {
    bucket: typeof inner.bucket === 'string' ? inner.bucket : undefined,
    stage: typeof inner.stage === 'string' ? inner.stage : undefined,
    message: typeof inner.message === 'string' && inner.message ? inner.message : 'The bucket could not be built.',
  }
}
