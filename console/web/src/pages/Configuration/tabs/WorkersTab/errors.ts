/**
 * Parser for `SCHEMA_INVALID` (and adjacent) errors returned by
 * `configuration::set`. The configuration worker delegates validation
 * to the Rust `jsonschema` crate, whose `Display` impl varies by error
 * kind — some carry an `instance_path`, some quote the offending key,
 * some give only a type narrative.
 *
 * We can't recover a JSON Pointer from every error, so the parser is
 * defensive: it always returns a human-readable `message`, and adds
 * `pointer` only when extraction succeeds. The `SchemaForm` keys its
 * per-field error display off `pointer`; the `SaveBar` always shows the
 * `message` so the operator never loses sight of why their save failed.
 *
 * The pattern matchers below are intentionally conservative — over-
 * matching would attach errors to the wrong field, which is worse
 * than not attaching them at all.
 */

export interface ParsedSetError {
  message: string
  /** JSON Pointer of the offending field, when extractable. */
  pointer?: string
  /** Original error `code` (e.g. `SCHEMA_INVALID`) when surfaced. */
  code?: string
}

/**
 * Match `at /a/b/c` or `at "/a/b/c"` at the end of a sentence — the
 * shape the engine and the `jsonschema` crate use for instance paths.
 */
const TRAILING_POINTER_RE = /\s+at\s+"?(\/[^\s"]*)"?\s*$/

/**
 * Match `Failed to validate '/a/b/c'` and friends — alternative phrasing
 * some validation paths use.
 */
const LEADING_PATH_RE = /^(?:.+?\s+)?at\s+"?(\/[^\s"]*)"?:\s*/

/**
 * Match `"key" is a required property` — emit a pointer to that key at
 * the root level when there's no other path information. This is best-
 * effort: the key could belong to a nested object, but the engine's
 * default formatting for `required` violations doesn't always include
 * the parent path. Better to surface nothing than a wrong path; we
 * only emit when no other pointer matched.
 */
const REQUIRED_PROPERTY_RE = /"([^"]+)" is a required property/

export function parseSetError(err: unknown): ParsedSetError {
  if (!err) return { message: 'unknown error' }

  let code: string | undefined
  let raw: string
  if (err instanceof Error) {
    raw = err.message
  } else if (typeof err === 'object') {
    const o = err as {
      code?: unknown
      message?: unknown
      error?: unknown
    }
    if (typeof o.code === 'string') code = o.code
    if (typeof o.message === 'string') raw = o.message
    else if (typeof o.error === 'string') raw = o.error
    else raw = String(err)
  } else {
    raw = String(err)
  }

  // Strip noise the engine prepends to every validation error.
  let message = raw
    .replace(/^Error:\s*/i, '')
    .replace(/^schema validation failed:\s*/i, '')
    .trim()

  let pointer: string | undefined

  const trailing = message.match(TRAILING_POINTER_RE)
  if (trailing) {
    pointer = trailing[1]
    message = message.replace(TRAILING_POINTER_RE, '').trim()
  } else {
    const leading = message.match(LEADING_PATH_RE)
    if (leading) {
      pointer = leading[1]
      message = message.replace(LEADING_PATH_RE, '').trim()
    }
  }

  if (!pointer) {
    const required = message.match(REQUIRED_PROPERTY_RE)
    if (required) {
      pointer = `/${required[1].replace(/~/g, '~0').replace(/\//g, '~1')}`
    }
  }

  return {
    message: message.length > 0 ? message : raw,
    pointer,
    code,
  }
}
