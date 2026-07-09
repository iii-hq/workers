/**
 * Pure classification helpers for `oneOf` / `anyOf` schemas, factored out of
 * `OneOfField` so the branch-shape logic is testable without a DOM.
 *
 * These recognize the two schemars encodings the configuration worker emits
 * that a naive "match by JSON type" union renderer gets wrong:
 *
 *   - `Option<T>` for a referenced/typed `T` → `anyOf: [{ $ref }, { type:
 *     null }]` (a *nullable* union, not a real choice).
 *   - an adjacently-tagged Rust enum → `oneOf` of objects each pinning a
 *     shared key to a distinct single-value `enum` (a *discriminated* union).
 */

import type { JsonSchema, JsonValue } from '../api'
import { schemaTypes } from './ref-resolver'

export function isPlainObject(
  value: JsonValue | undefined,
): value is { [key: string]: JsonValue } {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** A schema that matches only JSON `null` and nothing else. */
export function isNullSchema(schema: JsonSchema): boolean {
  const types = schemaTypes(schema)
  return (
    types.length === 1 &&
    types[0] === 'null' &&
    !Array.isArray(schema.enum) &&
    !schema.properties &&
    !schema.oneOf &&
    !schema.anyOf &&
    typeof schema.$ref !== 'string'
  )
}

/**
 * For a two-variant union that is exactly `[X, null]`, return the non-null
 * variant `X`; otherwise `null`. This is schemars's encoding of `Option<T>`
 * for a referenced/typed `T`.
 */
export function nullableUnionInner(variants: JsonSchema[]): JsonSchema | null {
  if (variants.length !== 2) return null
  const nullIdx = variants.findIndex(isNullSchema)
  if (nullIdx === -1) return null
  const inner = variants[1 - nullIdx]
  return isNullSchema(inner) ? null : inner
}

/**
 * Merge a nullable union's non-null branch into a single schema: keep the
 * branch's shape, add `"null"` to its `type` (so non-enum branches route to
 * `NullableField`), and carry the outer field's description/title/default.
 * An enum branch is left to `EnumField`, which already offers an "unset".
 */
export function mergeNullable(
  outer: JsonSchema,
  inner: JsonSchema,
): JsonSchema {
  const merged: JsonSchema = { ...inner }
  const innerTypes = schemaTypes(inner)
  if (innerTypes.length > 0 && !innerTypes.includes('null')) {
    const types = [...innerTypes, 'null']
    merged.type = types.length === 1 ? types[0] : types
  }
  if (typeof outer.description === 'string') {
    merged.description = outer.description
  }
  if (typeof outer.title === 'string' && typeof merged.title !== 'string') {
    merged.title = outer.title
  }
  if (outer.default !== undefined) merged.default = outer.default
  return merged
}

export interface Discriminator {
  key: string
  /** Discriminator value for each variant, index-aligned with `variants`. */
  values: string[]
}

/**
 * Detect a discriminated object union: every variant is an object that pins a
 * shared property to a distinct single-value `enum` (or `const`). Returns the
 * discriminator key and per-variant values, or `null` when the union isn't
 * shaped that way.
 */
export function discriminator(variants: JsonSchema[]): Discriminator | null {
  if (variants.length < 2) return null
  const propsList: (Record<string, JsonSchema> | null)[] = variants.map((v) => {
    // A variant explicitly typed as a non-object cannot be an object branch,
    // even if it carries a `properties` map — routing it here would emit an
    // object value (`{ [key]: value }`) for a string/number/etc. branch.
    const types = schemaTypes(v)
    if (types.length > 0 && !types.includes('object')) return null
    return v.properties &&
      typeof v.properties === 'object' &&
      !Array.isArray(v.properties)
      ? (v.properties as Record<string, JsonSchema>)
      : null
  })
  if (propsList.some((p) => p === null)) return null
  const allProps = propsList as Record<string, JsonSchema>[]

  for (const key of Object.keys(allProps[0])) {
    const values: string[] = []
    let ok = true
    for (const props of allProps) {
      const single = singleEnumString(props[key])
      if (single === null) {
        ok = false
        break
      }
      values.push(single)
    }
    if (!ok) continue
    // Values must be distinct, otherwise they cannot discriminate.
    if (new Set(values).size !== values.length) continue
    return { key, values }
  }
  return null
}

/** The lone string value a schema is pinned to (`enum: [X]` or `const: X`). */
export function singleEnumString(
  schema: JsonSchema | undefined,
): string | null {
  if (!schema) return null
  if (Array.isArray(schema.enum) && schema.enum.length === 1) {
    const first = (schema.enum as unknown[])[0]
    if (typeof first === 'string') return first
  }
  const constValue = (schema as { const?: unknown }).const
  if (typeof constValue === 'string') return constValue
  return null
}

/** Clone `schema` without one property (and without it in `required`/`title`). */
export function stripProperty(schema: JsonSchema, key: string): JsonSchema {
  const properties = {
    ...(schema.properties as Record<string, JsonSchema>),
  }
  delete properties[key]
  const next: JsonSchema = { ...schema, properties }
  if (Array.isArray(schema.required)) {
    next.required = (schema.required as string[]).filter((r) => r !== key)
  }
  // The branch title labels the discriminator option, not the sub-form.
  delete (next as { title?: unknown }).title
  return next
}

export function hasRenderableProps(schema: JsonSchema): boolean {
  const p = schema.properties
  return (
    !!p &&
    typeof p === 'object' &&
    !Array.isArray(p) &&
    Object.keys(p as object).length > 0
  )
}
