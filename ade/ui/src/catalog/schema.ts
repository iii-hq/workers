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

/* ── pre-flight check ───────────────────────────────────────────────── */

/**
 * What the schema can say for certain is wrong with a body, before the
 * call is made. A worker that is handed `"weeks": "test"` where it expects
 * an integer may answer with a serialization error, or may not answer at
 * all, so the obvious mistakes are caught here and read in the schema's
 * own words: `weeks must be an integer`.
 *
 * Only certainties are reported: a missing required field, a value of the
 * wrong JSON type, a value outside an `enum`, a value no `anyOf` branch
 * takes. Anything the schema leaves open (`$ref`, no `type`) passes, so a
 * loose schema never blocks a call the worker would have accepted.
 */
export function schemaProblems(schema: unknown, payload: unknown): string[] {
  const out: string[] = []
  check(schema, payload, '', out, 0)
  return out
}

const MAX_PROBLEMS = 6

function jsonType(value: unknown): string {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'array'
  return typeof value
}

/** Every type the schema allows, or `null` when it names none. */
function allowedTypes(schema: Record<string, unknown>): string[] | null {
  const t = schema.type
  if (typeof t === 'string') return [t]
  if (Array.isArray(t)) {
    const named = t.filter((x): x is string => typeof x === 'string')
    return named.length > 0 ? named : null
  }
  if (isRecord(schema.properties)) return ['object']
  if (schema.items !== undefined) return ['array']
  return null
}

function accepts(types: readonly string[], value: unknown): boolean {
  const actual = jsonType(value)
  for (const t of types) {
    if (t === actual) return true
    if (t === 'integer' && typeof value === 'number' && Number.isInteger(value)) return true
    if (t === 'number' && typeof value === 'number') return true
  }
  return false
}

function typeWord(types: readonly string[]): string {
  const words = types.map((t) => {
    if (t === 'integer') return 'an integer'
    if (t === 'array') return 'an array'
    if (t === 'object') return 'an object'
    if (t === 'null') return 'null'
    return `a ${t}`
  })
  return words.join(' or ')
}

function shown(value: unknown): string {
  const text = JSON.stringify(value)
  if (text === undefined) return String(value)
  return text.length > 24 ? `${text.slice(0, 23)}…` : text
}

function check(schema: unknown, value: unknown, path: string, out: string[], depth: number): void {
  if (!isRecord(schema) || depth > MAX_DEPTH || out.length >= MAX_PROBLEMS) return
  const name = path || 'The body'

  // `anyOf` / `oneOf`: the value must satisfy some branch. When every
  // branch names a type, the problem reads as one union; otherwise the
  // first branch's own complaint stands in.
  const branches = schema.anyOf ?? schema.oneOf
  if (Array.isArray(branches) && branches.length > 0 && schema.type === undefined) {
    const complaints: string[][] = []
    for (const branch of branches) {
      const scratch: string[] = []
      check(branch, value, path, scratch, depth + 1)
      if (scratch.length === 0) return
      complaints.push(scratch)
    }
    const named = branches.map((b) => (isRecord(b) ? allowedTypes(b) : null))
    if (named.every((t): t is string[] => t !== null)) {
      out.push(`${name} must be ${typeWord([...new Set(named.flat())])}, not ${shown(value)}`)
    } else {
      out.push(complaints[0][0])
    }
    return
  }

  const types = allowedTypes(schema)
  if (types && !accepts(types, value)) {
    out.push(`${name} must be ${typeWord(types)}, not ${shown(value)}`)
    return
  }
  if (Array.isArray(schema.enum) && schema.enum.length > 0) {
    const allowed = schema.enum as unknown[]
    if (!allowed.some((member) => JSON.stringify(member) === JSON.stringify(value))) {
      out.push(`${name} must be one of ${allowed.map(shown).join(', ')}`)
      return
    }
  }

  if (isRecord(value) && (types === null || types.includes('object'))) {
    const props = isRecord(schema.properties) ? schema.properties : {}
    const required = Array.isArray(schema.required)
      ? schema.required.filter((k): k is string => typeof k === 'string')
      : []
    for (const key of required) {
      if (value[key] === undefined) out.push(`${path ? `${path}.${key}` : key} is required`)
    }
    for (const [key, sub] of Object.entries(props)) {
      if (value[key] === undefined) continue
      check(sub, value[key], path ? `${path}.${key}` : key, out, depth + 1)
    }
  }

  if (Array.isArray(value) && isRecord(schema.items)) {
    value.forEach((item, i) => check(schema.items, item, `${path}[${i}]`, out, depth + 1))
  }
}
