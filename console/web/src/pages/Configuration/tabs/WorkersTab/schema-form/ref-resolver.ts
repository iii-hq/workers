/**
 * `$ref` resolution for draft-07 JSON schemas, including the common
 * `schemars`-generated wrapping pattern.
 *
 * Why this exists: draft-07 disallows sibling keywords next to `$ref`, so
 * crates like `schemars` express "this property has these defaults but is
 * really a `SomeType`" as `{ allOf: [{ $ref: "#/definitions/SomeType" }],
 * default: …, description: … }`. The form components want a clean
 * already-resolved schema with all keywords merged together — they should
 * never have to know `$ref` exists.
 *
 * Resolution rules:
 *   1. `{ $ref: "#/…" }` → the pointed-to schema (recursively resolved).
 *   2. `{ allOf: [{ $ref: "…" }], …siblings }` → resolve the ref, then
 *      shallow-merge the siblings on top (so `default`, `description`,
 *      `title` from the wrapper win over the referenced schema).
 *   3. `{ allOf: [a, b, …] }` with no single-$ref short-circuit → kept
 *      as-is; full `allOf` intersection is not implemented because the
 *      configuration worker's schemas never use it for anything beyond
 *      the wrapping pattern above.
 *
 * External `$ref`s (anything that doesn't start with `#/`) and broken
 * pointers resolve to `{}` so the dispatcher falls back to its "unknown
 * type" branch rather than crashing.
 *
 * Cycles are guarded with a `seen` set so a recursive schema does not
 * blow the stack; the cycle ends in a stub schema carrying a description.
 */

import type { JsonSchema, JsonValue } from '../api'

export interface ResolveContext {
  root: JsonSchema
  /** Internal — accumulated set of pointers already on the resolution stack. */
  seen?: ReadonlySet<string>
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * Resolve `schema` against `ctx.root`. Returns a fresh schema object —
 * never mutates the input. Idempotent: calling `resolveSchema` on an
 * already-resolved schema returns it unchanged.
 */
export function resolveSchema(
  schema: JsonSchema,
  ctx: ResolveContext,
): JsonSchema {
  // Pattern 2: `allOf` wrapper with a single $ref + sibling defaults.
  if (
    Array.isArray(schema.allOf) &&
    schema.allOf.length === 1 &&
    isObject(schema.allOf[0]) &&
    typeof (schema.allOf[0] as JsonSchema).$ref === 'string'
  ) {
    const refTarget = (schema.allOf[0] as JsonSchema).$ref as string
    const resolved = resolveRef(refTarget, ctx)
    const { allOf: _allOf, ...siblings } = schema
    return { ...resolved, ...siblings }
  }

  // Pattern 1: direct `$ref`.
  if (typeof schema.$ref === 'string') {
    return resolveRef(schema.$ref, ctx)
  }

  return schema
}

/**
 * Resolve a `#/…` JSON Pointer against `ctx.root` and recursively walk
 * any nested refs. Returns `{}` for unsupported or broken refs.
 */
function resolveRef(ref: string, ctx: ResolveContext): JsonSchema {
  if (!ref.startsWith('#/')) {
    return {}
  }
  const seen = ctx.seen ?? new Set<string>()
  if (seen.has(ref)) {
    return { description: `cycle in $ref: ${ref}` }
  }
  const segments = ref.slice(2).split('/')
  let current: unknown = ctx.root
  for (const segment of segments) {
    const decoded = decodePointerSegment(segment)
    if (isObject(current)) {
      current = current[decoded]
    } else {
      return {}
    }
  }
  if (!isObject(current)) {
    return {}
  }
  const nextSeen = new Set(seen)
  nextSeen.add(ref)
  return resolveSchema(current as JsonSchema, {
    root: ctx.root,
    seen: nextSeen,
  })
}

function decodePointerSegment(segment: string): string {
  return segment.replace(/~1/g, '/').replace(/~0/g, '~')
}

/* ---------------------------------------------------------------------- */
/*  Schema-shape helpers used by the dispatcher                          */
/* ---------------------------------------------------------------------- */

/**
 * Schema `type` keyword as a normalized string array. Draft-07 allows
 * either a single string or an array; we always work with the array
 * form so downstream code can do `types.includes('null')` without
 * separate branches.
 */
export function schemaTypes(schema: JsonSchema): string[] {
  const t = schema.type
  if (typeof t === 'string') return [t]
  if (Array.isArray(t))
    return t.filter((x): x is string => typeof x === 'string')
  return []
}

/**
 * True when the schema's `type` includes `"null"` alongside other types
 * — i.e. nullable. The dispatcher uses this to wrap the inner field in
 * a `NullableField` toggle.
 */
export function isNullable(schema: JsonSchema): boolean {
  const types = schemaTypes(schema)
  return types.includes('null') && types.length > 1
}

/**
 * Return a clone of `schema` with `null` stripped from `type`. Used by
 * `NullableField` to render the inner field as if the nullable wrapper
 * weren't there. Returns the input schema unchanged (reference-equal)
 * when there is no `null` to strip — keeps render-paths cheap and
 * makes the function safe to call unconditionally.
 */
export function withoutNull(schema: JsonSchema): JsonSchema {
  const original = schemaTypes(schema)
  if (!original.includes('null')) return schema
  const types = original.filter((t) => t !== 'null')
  if (types.length === 0) return schema
  const next: JsonSchema = { ...schema }
  next.type = types.length === 1 ? types[0] : types
  return next
}

/**
 * True when a schema renders as a single inline control (string, number,
 * integer, boolean, or any `enum`) rather than a multi-row block. Mirrors
 * the precedence in `FieldDispatch`: `enum` wins, `oneOf`/`anyOf` and
 * nullable unions are never inline, then the primitive types qualify.
 *
 * `ArrayField` / `DictionaryField` use this to decide between a compact
 * single-input row (env-file style) and a stacked block for complex
 * values. The schema is resolved against `root` first so a `$ref` to an
 * enum or primitive is classified correctly.
 */
export function isInlineLeaf(schema: JsonSchema, root: JsonSchema): boolean {
  const resolved = resolveSchema(schema, { root })
  if (Array.isArray(resolved.enum)) return true
  if (Array.isArray(resolved.oneOf) || Array.isArray(resolved.anyOf)) {
    return false
  }
  if (isNullable(resolved)) return false
  const primary = schemaTypes(resolved)[0]
  return (
    primary === 'string' ||
    primary === 'number' ||
    primary === 'integer' ||
    primary === 'boolean'
  )
}

/**
 * Pick a reasonable default value for a schema. Uses `schema.default`
 * when present; otherwise falls back to a value matching the declared
 * type (empty string, 0, false, [], {}, null).
 */
export function schemaDefault(schema: JsonSchema): JsonValue {
  if (schema.default !== undefined) {
    return schema.default as JsonValue
  }
  const types = schemaTypes(schema)
  if (types.includes('string')) return ''
  if (types.includes('number') || types.includes('integer')) return 0
  if (types.includes('boolean')) return false
  if (types.includes('array')) return []
  if (types.includes('object')) return {}
  return null
}
