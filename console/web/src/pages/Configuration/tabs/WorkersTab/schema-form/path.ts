/**
 * Lens-style getter/setter for `JsonValue` trees by structured path. The
 * schema form components receive a path slice + the loaded value root and
 * use these helpers to read/write without re-implementing the "build the
 * new tree at every level" boilerplate.
 *
 * A `Path` is an array of tokens: strings address object keys, numbers
 * address array indices. We keep two representations (structured `Path`
 * and RFC 6901 JSON Pointer) because the structured form is what the
 * components manipulate, while the pointer form is what `SCHEMA_INVALID`
 * server errors come back as.
 *
 * All operations are pure and never mutate the input — they always return
 * a new tree, so React state updates work safely.
 */

import type { JsonValue } from '../api'

export type PathToken = string | number
export type Path = readonly PathToken[]

function isPlainObject(
  value: JsonValue | undefined,
): value is { [key: string]: JsonValue } {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * Read the value at `path`. Returns `undefined` when any intermediate
 * step is missing or has the wrong shape — callers should treat
 * `undefined` and `null` the same way unless they specifically need to
 * distinguish "explicitly null" from "missing key".
 */
export function pathGet(
  value: JsonValue | undefined,
  path: Path,
): JsonValue | undefined {
  let current: JsonValue | undefined = value
  for (const token of path) {
    if (current === null || current === undefined) return undefined
    if (typeof token === 'number') {
      if (!Array.isArray(current)) return undefined
      current = current[token]
    } else {
      if (!isPlainObject(current)) return undefined
      current = current[token]
    }
  }
  return current
}

/**
 * Return a new tree with the value at `path` replaced by `next`. Missing
 * intermediate objects/arrays are created on the fly so the caller can
 * write into a deeply-nested path that doesn't fully exist yet.
 *
 * When the head token is a numeric index past the end of the array, the
 * array is padded with `null` up to that index so the resulting tree is
 * still a valid JSON value.
 */
export function pathSet(
  value: JsonValue | undefined,
  path: Path,
  next: JsonValue,
): JsonValue {
  if (path.length === 0) return next
  const [head, ...rest] = path
  if (typeof head === 'number') {
    const source = Array.isArray(value) ? value : []
    const arr: JsonValue[] = source.slice()
    while (arr.length <= head) arr.push(null)
    arr[head] = pathSet(arr[head], rest, next)
    return arr
  }
  const source = isPlainObject(value) ? value : {}
  const obj: { [key: string]: JsonValue } = { ...source }
  obj[head] = pathSet(obj[head], rest, next)
  return obj
}

/**
 * Return a new tree with the value at `path` removed. For object keys
 * the key is `delete`d; for array indices the element is `splice`d out.
 * Returns the input unchanged if any intermediate step is missing.
 */
export function pathUnset(
  value: JsonValue | undefined,
  path: Path,
): JsonValue | undefined {
  if (path.length === 0) return undefined
  const [head, ...rest] = path
  if (rest.length === 0) {
    if (typeof head === 'number') {
      if (!Array.isArray(value)) return value
      const arr: JsonValue[] = value.slice()
      arr.splice(head, 1)
      return arr
    }
    if (!isPlainObject(value)) return value
    const obj: { [key: string]: JsonValue } = { ...value }
    delete obj[head]
    return obj
  }
  if (typeof head === 'number') {
    if (!Array.isArray(value)) return value
    const arr: JsonValue[] = value.slice()
    arr[head] = pathUnset(arr[head], rest) as JsonValue
    return arr
  }
  if (!isPlainObject(value)) return value
  const obj: { [key: string]: JsonValue } = { ...value }
  obj[head] = pathUnset(obj[head], rest) as JsonValue
  return obj
}

/**
 * Encode a structured path as an RFC 6901 JSON Pointer. Used when we
 * need to talk to the server (the configuration worker returns paths
 * in pointer form on `SCHEMA_INVALID`).
 *
 * `~` becomes `~0`, `/` becomes `~1` per the spec.
 */
export function pathToPointer(path: Path): string {
  if (path.length === 0) return ''
  return (
    '/' +
    path
      .map((token) => String(token).replace(/~/g, '~0').replace(/\//g, '~1'))
      .join('/')
  )
}

/**
 * Decode an RFC 6901 JSON Pointer back into a structured path. Numeric
 * tokens that look like array indices are returned as numbers so the
 * `pathSet` array branch fires; this matches how `pathGet` walks indices.
 */
export function pointerToPath(pointer: string): Path {
  if (pointer === '') return []
  if (!pointer.startsWith('/')) {
    throw new Error(`invalid json pointer: ${pointer}`)
  }
  return pointer
    .slice(1)
    .split('/')
    .map((token) => {
      const decoded = token.replace(/~1/g, '/').replace(/~0/g, '~')
      if (/^(0|[1-9][0-9]*)$/.test(decoded)) {
        return Number.parseInt(decoded, 10)
      }
      return decoded
    })
}
