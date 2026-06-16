/**
 * Pure helpers for `oneOf` / `anyOf` variant handling, split out of
 * `OneOfField` so the matching, labelling, and default-on-switch logic can
 * be unit-tested without rendering React.
 *
 * The interesting case is schemars' adjacently/internally tagged enums
 * (`#[serde(tag = "name", content = "config")]`), which serialize to a
 * `oneOf` of objects that each carry a single-value enum "tag" property
 * (e.g. `name: { enum: ["bridge"] }`). Matching, labelling, and switching
 * all key off that tag so a stored `{ name: "bridge", … }` round-trips to
 * the right variant.
 */

import type { JsonSchema, JsonValue } from '../api'
import { resolveSchema, schemaDefault, schemaTypes } from './ref-resolver'

/**
 * True when a variant is a single-value string enum (schemars's
 * representation of a unit Rust enum variant under `serde(rename_all)`).
 * `OneOfField` collapses a union of these into one flat `enum` dropdown.
 */
export function isSingleStringEnumVariant(variant: JsonSchema): boolean {
  if (!Array.isArray(variant.enum) || variant.enum.length !== 1) return false
  const types = schemaTypes(variant)
  // Accept either explicit `type: string` or an unspecified type with a
  // string-typed enum value (some schemars outputs omit `type`).
  if (types.length > 0 && !types.includes('string')) return false
  return typeof (variant.enum as unknown[])[0] === 'string'
}

/**
 * Human label for a variant in the picker. Prefers an explicit `title`,
 * then a single-value enum, then a tagged object's tag value (e.g. an
 * adapter `name` of "fs" / "bridge") instead of the useless "object", then
 * the JSON type(s).
 */
export function variantLabel(variant: JsonSchema, idx: number): string {
  if (typeof variant.title === 'string') return variant.title
  if (Array.isArray(variant.enum) && variant.enum.length === 1) {
    const single = (variant.enum as unknown[])[0]
    if (typeof single === 'string') return single
  }
  const tag = objectTagValue(variant)
  if (typeof tag === 'string') return tag
  const types = schemaTypes(variant)
  if (types.length > 0) return types.join(' | ')
  return `variant ${idx + 1}`
}

/**
 * Best-effort match of the current value to one of the variant schemas.
 * Returns the index of the first matching variant, or 0 when no variant
 * fits (so the dispatcher always has something to render).
 *
 * Discriminated variants win first: when the value sets a variant's
 * single-value enum tag (e.g. `{ name: "bridge" }`) we select that variant,
 * so two object variants don't both collapse onto the first. Otherwise we
 * fall back to structural (type-based) matching for genuinely heterogeneous
 * unions.
 */
export function matchVariantIndex(
  variants: JsonSchema[],
  value: JsonValue | undefined,
): number {
  const tagged = variants.findIndex((v) => valueMatchesTags(value, v))
  if (tagged !== -1) return tagged
  for (let i = 0; i < variants.length; i++) {
    if (valueMatchesSchema(value, variants[i])) return i
  }
  return 0
}

/**
 * Default value for a variant the operator just switched to. Unlike the
 * shallow [`schemaDefault`], this recurses into object properties so a
 * discriminated variant comes back fully formed — single-value-enum tags
 * get their constant (so `matchVariantIndex` re-selects this variant) and
 * nested `$ref` content (e.g. an adapter `config`) gets its own defaults.
 */
export function variantDefault(
  schema: JsonSchema,
  root: JsonSchema,
): JsonValue {
  const resolved = resolveSchema(schema, { root })
  if (resolved.default !== undefined) return resolved.default as JsonValue
  const single = singleEnumValue(resolved)
  if (single !== undefined) return single
  if (
    schemaTypes(resolved).includes('object') &&
    isPlainObject(resolved.properties)
  ) {
    const out: Record<string, JsonValue> = {}
    for (const [key, propSchema] of Object.entries(resolved.properties)) {
      if (!isPlainObject(propSchema)) continue
      out[key] = variantDefault(propSchema as JsonSchema, root)
    }
    return out
  }
  return schemaDefault(resolved)
}

/**
 * The single-value enum of a schema, if it has one (schemars renders a
 * unit-like tag as `{ enum: ["fs"] }`). Returns `undefined` otherwise.
 */
function singleEnumValue(schema: JsonSchema): JsonValue | undefined {
  if (Array.isArray(schema.enum) && schema.enum.length === 1) {
    return schema.enum[0] as JsonValue
  }
  return undefined
}

/** The first single-value-enum tag of an object schema's properties. */
function objectTagValue(schema: JsonSchema): JsonValue | undefined {
  if (!isPlainObject(schema.properties)) return undefined
  for (const propSchema of Object.values(schema.properties)) {
    if (!isPlainObject(propSchema)) continue
    const tag = singleEnumValue(propSchema as JsonSchema)
    if (tag !== undefined) return tag
  }
  return undefined
}

/**
 * True when every single-value-enum tag property of an object variant is
 * present and equal in `value`. Returns false when the variant carries no
 * tags (so the caller falls back to structural matching) or the value is
 * not an object.
 */
function valueMatchesTags(
  value: JsonValue | undefined,
  schema: JsonSchema,
): boolean {
  if (!isPlainObject(value) || !isPlainObject(schema.properties)) return false
  let tagCount = 0
  for (const [key, propSchema] of Object.entries(schema.properties)) {
    if (!isPlainObject(propSchema)) continue
    const tag = singleEnumValue(propSchema as JsonSchema)
    if (tag === undefined) continue
    tagCount++
    if (!deepEqual((value as Record<string, JsonValue>)[key], tag)) {
      return false
    }
  }
  return tagCount > 0
}

function valueMatchesSchema(
  value: JsonValue | undefined,
  schema: JsonSchema,
): boolean {
  if (Array.isArray(schema.enum)) {
    return (schema.enum as JsonValue[]).some((v) => deepEqual(v, value))
  }
  const types = schemaTypes(schema)
  if (types.length === 0) return false
  const actual = jsonType(value)
  return types.includes(actual)
}

function jsonType(value: JsonValue | undefined): string {
  if (value === null || value === undefined) return 'null'
  if (Array.isArray(value)) return 'array'
  if (Number.isInteger(value as number)) return 'integer'
  return typeof value
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function deepEqual(
  a: JsonValue | undefined,
  b: JsonValue | undefined,
): boolean {
  if (a === b) return true
  if (a === null || b === null) return false
  if (typeof a !== typeof b) return false
  if (Array.isArray(a) !== Array.isArray(b)) return false
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false
    return a.every((x, i) => deepEqual(x, b[i]))
  }
  if (typeof a === 'object' && typeof b === 'object') {
    const ak = Object.keys(a as object)
    const bk = Object.keys(b as object)
    if (ak.length !== bk.length) return false
    return ak.every((k) =>
      deepEqual(
        (a as Record<string, JsonValue>)[k],
        (b as Record<string, JsonValue>)[k],
      ),
    )
  }
  return false
}
