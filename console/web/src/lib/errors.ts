/**
 * One readable string out of whatever a rejected `iii.trigger` throws.
 *
 * The browser SDK rejects with the wire error *object*, not an `Error`, so the
 * usual `err instanceof Error ? err.message : String(err)` renders every failed
 * call as `[object Object]` — which is worse than useless, because it hides the
 * one thing the user needs: a stack whose worker predates the function answers
 * `{code: "function_not_found", message: "Function x::y not found"}`, and that
 * message is the whole diagnosis.
 *
 * The envelope also nests: the transport says `invocation_failed` and buries
 * the useful part inside `message` as `handler error: {"code":…,"message":…}`.
 * Unwrap to the innermost message and prefix the handler's own code, so a
 * duplicate prompt reads `D214: prompt "test" already exists.` rather than the
 * transport's generic failure.
 *
 * Not every handler error is JSON, though — iii-directory's are plain prose
 * strings like `D214 invalid_input: …`, so there is no `{` for the unwrap to
 * find and the SDK's `handler error: ` prefix survives unstripped. Peel that
 * prefix off too so the code+message still reads clean.
 *
 * Ported from `database/ui/src/lib/errors.ts` — same SDK, same bug. A shared
 * home already exists (both packages depend on `@iii-dev/console-ui`); this
 * copy hasn't been consolidated into it yet.
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
    const { code, inner_code, message: unwrapped } = unwrap(body)
    // iii-directory's handler errors are plain strings (`D214 invalid_input:
    // …`), not JSON, so `unwrap` above has nothing to peel — the SDK's own
    // `handler error: ` prefix is still there. Strip it so the handler's own
    // code+message reads clean instead of doubling up with `label` below.
    const message = unwrapped?.replace(/^handler error: /, '')
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
