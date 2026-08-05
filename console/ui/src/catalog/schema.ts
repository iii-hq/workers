/**
 * JSON Schema → starting request body. The engine registers draft-07
 * schemas for most functions, so the invoke editor can open on the real
 * field names instead of an empty object.
 *
 * Deliberately shallow-minded: it fills the shape, never plausible values.
 * `default` and the first `enum` member are the only values it invents,
 * because those are the schema's own words. Anything it cannot read becomes
 * `{}` and the operator types the body themselves.
 */

const MAX_DEPTH = 4

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** `type` may be a string or a union array (`["string", "null"]`). */
function primaryType(schema: Record<string, unknown>): string | undefined {
  const t = schema.type
  if (typeof t === 'string') return t
  if (Array.isArray(t)) {
    const named = t.find((x) => typeof x === 'string' && x !== 'null')
    if (typeof named === 'string') return named
  }
  // A bare `properties`/`items` with no `type` is still an object/array.
  if (isRecord(schema.properties)) return 'object'
  if (schema.items !== undefined) return 'array'
  return undefined
}

function sample(schema: unknown, depth: number): unknown {
  if (!isRecord(schema) || depth > MAX_DEPTH) return null
  if (schema.default !== undefined) return schema.default
  if (Array.isArray(schema.enum) && schema.enum.length > 0)
    return schema.enum[0]

  const composed = schema.oneOf ?? schema.anyOf ?? schema.allOf
  if (Array.isArray(composed) && composed.length > 0) {
    return sample(composed[0], depth + 1)
  }

  switch (primaryType(schema)) {
    case 'object': {
      const props = isRecord(schema.properties) ? schema.properties : {}
      const required = Array.isArray(schema.required)
        ? schema.required.filter((k): k is string => typeof k === 'string')
        : []
      const keys = Object.keys(props)
      // Required fields first, then the rest in declaration order — the
      // operator reads the fields they must fill without scrolling.
      const ordered = [
        ...keys.filter((k) => required.includes(k)),
        ...keys.filter((k) => !required.includes(k)),
      ]
      const out: Record<string, unknown> = {}
      for (const key of ordered) out[key] = sample(props[key], depth + 1)
      return out
    }
    case 'array':
      return []
    case 'string':
      return ''
    case 'number':
    case 'integer':
      return 0
    case 'boolean':
      return false
    default:
      return null
  }
}

/** Pretty-printed starting body for the invoke editor. */
export function templateFromSchema(schema: unknown): string {
  const value = sample(schema, 0)
  if (!isRecord(value) || Object.keys(value).length === 0) return '{}'
  return JSON.stringify(value, null, 2)
}

/** Pretty JSON for the read-only panes; non-JSON values degrade to text. */
export function pretty(value: unknown): string {
  if (value === undefined) return ''
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return String(value)
  }
}

/**
 * The namespace a function id belongs to: everything before the first `::`.
 * Ids without a namespace group under `other` so no row goes missing.
 */
export function namespaceOf(functionId: string): string {
  const cut = functionId.indexOf('::')
  return cut > 0 ? functionId.slice(0, cut) : 'other'
}

/** Group label ordering: alphabetical, with the `other` bucket last. */
export function compareGroups(a: string, b: string): number {
  if (a === 'other') return 1
  if (b === 'other') return -1
  return a.localeCompare(b)
}
