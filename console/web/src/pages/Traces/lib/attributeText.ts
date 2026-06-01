/**
 * Coerce an untrusted OTel attribute value into renderable text.
 *
 * Span attributes cross the engine RPC boundary typed as `unknown` and are
 * producer-controlled — a value declared as a string (e.g.
 * `exception.stacktrace`, `error.message`) can arrive as an array, object,
 * or number. Passing such a value straight to a String method like
 * `.split('\n')`, or rendering an object as a React child, throws and
 * (under the page ErrorBoundary) blanks the whole Traces view.
 *
 * Returns `undefined` for nullish input so callers can keep using
 * truthiness checks; everything else becomes a string. JSON is used for
 * arrays/objects; unserializable values fall back to `String(value)`.
 */
export function attributeText(value: unknown): string | undefined {
  if (value == null) return undefined
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value)
  }
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}
