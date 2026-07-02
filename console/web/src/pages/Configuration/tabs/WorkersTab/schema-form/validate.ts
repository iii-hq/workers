/**
 * Client-side configuration validation that mirrors the engine's
 * configuration worker, so the console can block an invalid save and show the
 * error inline instead of round-tripping to the server.
 *
 * The hard part is NOT the JSON Schema keywords — it's env templates. The
 * engine validates the APPLIED (env-expanded + type-coerced) value, not the raw
 * `${VAR}` template: it substitutes a lone placeholder and re-parses the result
 * as a YAML scalar, so `port: ${HTTP_PORT:3111}` validates as the integer 3111.
 * This module reproduces that contract:
 *
 *   - a lone `${VAR:default}` placeholder → its coerced default is validated
 *     (the browser has no process env, so the default is the best we can do);
 *   - a lone `${VAR}` with no default → unresolvable here → that leaf is left
 *     unvalidated (the engine resolves it from env at runtime — we must not
 *     falsely reject it, and key-presence still satisfies `required`);
 *   - a mixed/embedded template (`redis://${HOST}`) → stays a string; only its
 *     type is checked, not pattern/length (the runtime value differs).
 *
 * IMPORTANT — keep `coerceScalar` in lock-step with the engine's
 * `expand_string` in
 * `iii/engine/src/workers/configuration/store.rs`. Both re-parse a substituted
 * lone placeholder as a YAML 1.2 scalar; if they drift, the console and engine
 * will disagree about which env-driven values are valid.
 *
 * This is a deliberately conservative subset of JSON Schema — it covers the
 * keywords schemars emits and these configs use (type, required, enum, const,
 * minimum/maximum/exclusive*, minLength/maxLength, pattern, minItems/maxItems,
 * properties, additionalProperties, items, oneOf/anyOf, $ref/allOf-wrapper).
 * Out of scope (the server stays the final authority): format assertions,
 * multipleOf, uniqueItems, dependencies, if/then/else. For oneOf/anyOf we
 * validate only the *active* branch (the one the form renders), to keep errors
 * attributable to what the operator can see, rather than scattering them across
 * branches they never chose.
 */

import type { JsonSchema, JsonValue } from '../api'
import { parseTemplate } from './env-lexical/env-template'
import { discriminator, nullableUnionInner } from './oneof-shape'
import { type Path, pathToPointer } from './path'
import {
  isNullable,
  resolveSchema,
  schemaTypes,
  withoutNull,
} from './ref-resolver'
import { matchVariantIndex } from './variant-match'

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

const BOOL_TRUE = new Set(['true', 'True', 'TRUE'])
const BOOL_FALSE = new Set(['false', 'False', 'FALSE'])
const NULL_TOKENS = new Set(['null', 'Null', 'NULL', '~'])
// Plain base-10 number (YAML 1.2 core scalar). Mirrors what serde_yaml resolves
// a substituted scalar to: integers, decimals, and scientific notation. Bare
// words, IPs (`127.0.0.1`), `on`/`off`, hex, etc. fall through to string.
const NUMBER_RE = /^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/

/**
 * Coerce the substituted text of a lone placeholder to the scalar the engine
 * would produce (`"8080"` → `8080`, `"true"` → `true`, `"localhost"` →
 * `"localhost"`). Keep in lock-step with `store.rs::expand_string`.
 */
export function coerceScalar(text: string): JsonValue {
  if (text.length === 0) return text
  if (BOOL_TRUE.has(text)) return true
  if (BOOL_FALSE.has(text)) return false
  if (NULL_TOKENS.has(text)) return null
  if (NUMBER_RE.test(text)) {
    const n = Number(text)
    if (Number.isFinite(n)) return n
  }
  return text
}

type LeafResolution =
  | { kind: 'value'; resolved: JsonValue }
  | { kind: 'unresolved' }

/**
 * Resolve a leaf to the value the engine would validate. See the module
 * header for the rules. Non-strings pass through unchanged.
 */
export function resolveLeafForValidation(value: JsonValue): LeafResolution {
  if (typeof value !== 'string') return { kind: 'value', resolved: value }
  const segments = parseTemplate(value)
  const placeholders = segments.filter((s) => s.kind === 'placeholder')
  if (placeholders.length === 0) {
    // A plain literal string.
    return { kind: 'value', resolved: value }
  }
  if (segments.length === 1 && segments[0].kind === 'placeholder') {
    // A lone `${...}` placeholder.
    const { defaultValue } = segments[0]
    if (defaultValue === null) return { kind: 'unresolved' }
    if (defaultValue === '') return { kind: 'value', resolved: '' }
    return { kind: 'value', resolved: coerceScalar(defaultValue) }
  }
  // Mixed / embedded template — stays a string at runtime.
  return { kind: 'value', resolved: value }
}

/** True when a (resolved) value is still a string carrying `${...}` segments. */
function stillTemplated(value: JsonValue): boolean {
  return (
    typeof value === 'string' &&
    parseTemplate(value).some((s) => s.kind === 'placeholder')
  )
}

type JsonTypeName =
  | 'null'
  | 'boolean'
  | 'integer'
  | 'number'
  | 'string'
  | 'array'
  | 'object'

function jsonTypeOf(value: JsonValue): JsonTypeName {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'array'
  const t = typeof value
  if (t === 'number') return Number.isInteger(value) ? 'integer' : 'number'
  if (t === 'boolean') return 'boolean'
  if (t === 'object') return 'object'
  return 'string'
}

function typeAllowed(value: JsonValue, types: string[]): boolean {
  if (types.length === 0) return true // no `type` constraint to enforce
  const actual = jsonTypeOf(value)
  if (types.includes(actual)) return true
  // An integer satisfies a `number` schema.
  if (actual === 'integer' && types.includes('number')) return true
  return false
}

function typeMessage(types: string[]): string {
  if (types.length === 1) {
    switch (types[0]) {
      case 'integer':
        return 'must be an integer'
      case 'number':
        return 'must be a number'
      case 'boolean':
        return 'must be a boolean'
      case 'string':
        return 'must be a string'
      case 'object':
        return 'must be an object'
      case 'array':
        return 'must be an array'
    }
  }
  return `must be of type: ${types.join(', ')}`
}

interface Ctx {
  root: JsonSchema
  out: Map<string, string>
}

/** Record the first error seen for a pointer (later ones are usually noise). */
function emit(ctx: Ctx, path: Path, message: string): void {
  const ptr = pathToPointer(path)
  if (!ctx.out.has(ptr)) ctx.out.set(ptr, message)
}

/**
 * Validate `value` against `schema`, returning a map of JSON Pointer → message
 * keyed exactly like `errorForField` (`FieldShell.tsx`) reads it, so errors
 * render inline on the owning field. An empty map means "valid".
 */
export function validateConfig(
  value: JsonValue,
  schema: JsonSchema,
): Map<string, string> {
  const ctx: Ctx = { root: schema, out: new Map() }
  validateNode(schema, value, [], ctx)
  return ctx.out
}

function validateNode(
  schemaIn: JsonSchema,
  value: JsonValue | undefined,
  path: Path,
  ctx: Ctx,
): void {
  if (value === undefined) return // absent; `required` is enforced by the parent
  const schema = resolveSchema(schemaIn, { root: ctx.root })

  if (Array.isArray(schema.enum)) {
    validateEnum(schema, value, path, ctx)
    return
  }
  if ('const' in schema) {
    validateConst(schema, value, path, ctx)
    return
  }

  const variants = (schema.oneOf ?? schema.anyOf) as JsonSchema[] | undefined
  if (Array.isArray(variants)) {
    validateUnion(variants, value, path, ctx)
    return
  }

  if (isNullable(schema)) {
    if (value === null) return // the `null` branch is satisfied
    validateNode(withoutNull(schema), value, path, ctx)
    return
  }

  const types = schemaTypes(schema)
  const primary = types[0]

  if (
    primary === 'object' ||
    (types.length === 0 && isObject(schema.properties))
  ) {
    validateObject(schema, value, path, ctx)
    return
  }
  if (primary === 'array') {
    validateArray(schema, value, path, ctx)
    return
  }
  if (
    primary === 'string' ||
    primary === 'number' ||
    primary === 'integer' ||
    primary === 'boolean'
  ) {
    validateLeaf(schema, value, path, ctx, types)
    return
  }
  // Unknown / untyped schema — nothing to assert (server stays authoritative).
}

function validateEnum(
  schema: JsonSchema,
  value: JsonValue,
  path: Path,
  ctx: Ctx,
): void {
  const res = resolveLeafForValidation(value)
  if (res.kind === 'unresolved') return
  if (stillTemplated(res.resolved)) return
  const allowed = schema.enum as JsonValue[]
  if (!allowed.some((a) => a === res.resolved)) {
    emit(
      ctx,
      path,
      `must be one of: ${allowed.map((a) => String(a)).join(', ')}`,
    )
  }
}

function validateConst(
  schema: JsonSchema,
  value: JsonValue,
  path: Path,
  ctx: Ctx,
): void {
  const res = resolveLeafForValidation(value)
  if (res.kind === 'unresolved') return
  if (stillTemplated(res.resolved)) return
  if (res.resolved !== (schema.const as JsonValue)) {
    emit(ctx, path, `must be ${JSON.stringify(schema.const)}`)
  }
}

function validateUnion(
  variants: JsonSchema[],
  value: JsonValue,
  path: Path,
  ctx: Ctx,
): void {
  // `Option<T>` → `[T, null]`: validate the inner branch unless the value is null.
  const inner = nullableUnionInner(variants)
  if (inner) {
    if (value === null) return
    validateNode(inner, value, path, ctx)
    return
  }
  // Discriminated union (adjacently-tagged enum): pick by the tag the value
  // carries, so we validate the branch the form actually renders.
  const disc = discriminator(variants)
  if (disc && isObject(value)) {
    const tag = (value as Record<string, JsonValue>)[disc.key]
    const idx = typeof tag === 'string' ? disc.values.indexOf(tag) : -1
    if (idx === -1) {
      emit(ctx, path, `must be one of: ${disc.values.join(', ')}`)
      return
    }
    validateNode(variants[idx], value, path, ctx)
    return
  }
  // Heterogeneous union — validate the structurally-matched branch.
  validateNode(variants[matchVariantIndex(variants, value)], value, path, ctx)
}

function validateObject(
  schema: JsonSchema,
  value: JsonValue,
  path: Path,
  ctx: Ctx,
): void {
  if (!isObject(value)) {
    emit(ctx, path, 'must be an object')
    return
  }
  const obj = value as Record<string, JsonValue>

  // `required` is satisfied by key presence on the raw draft — an unresolved
  // `${VAR}` leaf is still present, so it counts.
  if (Array.isArray(schema.required)) {
    for (const key of schema.required as string[]) {
      if (obj[key] === undefined) {
        emit(ctx, [...path, key], `"${key}" is a required property`)
      }
    }
  }

  if (isObject(schema.properties)) {
    const props = schema.properties as Record<string, JsonSchema>
    for (const [key, propSchema] of Object.entries(props)) {
      if (!isObject(propSchema)) continue
      validateNode(propSchema as JsonSchema, obj[key], [...path, key], ctx)
    }
    return
  }

  // Dictionary: `additionalProperties` is a schema applied to every value.
  const ap = schema.additionalProperties
  if (isObject(ap)) {
    for (const [key, child] of Object.entries(obj)) {
      validateNode(ap as JsonSchema, child, [...path, key], ctx)
    }
  }
}

function validateArray(
  schema: JsonSchema,
  value: JsonValue,
  path: Path,
  ctx: Ctx,
): void {
  if (!Array.isArray(value)) {
    emit(ctx, path, 'must be an array')
    return
  }
  if (typeof schema.minItems === 'number' && value.length < schema.minItems) {
    emit(ctx, path, `must have at least ${schema.minItems} item(s)`)
  }
  if (typeof schema.maxItems === 'number' && value.length > schema.maxItems) {
    emit(ctx, path, `must have at most ${schema.maxItems} item(s)`)
  }
  if (isObject(schema.items)) {
    const items = schema.items as JsonSchema
    value.forEach((el, i) => {
      validateNode(items, el, [...path, i], ctx)
    })
  }
}

function validateLeaf(
  schema: JsonSchema,
  value: JsonValue,
  path: Path,
  ctx: Ctx,
  types: string[],
): void {
  const res = resolveLeafForValidation(value)
  if (res.kind === 'unresolved') return
  const v = res.resolved

  if (!typeAllowed(v, types)) {
    emit(ctx, path, typeMessage(types))
    return
  }

  // An embedded/mixed template stays a string at runtime; only its type is
  // knowable client-side, so skip value-shape constraints below.
  if (stillTemplated(v)) return

  if (typeof v === 'number') {
    if (typeof schema.minimum === 'number' && v < schema.minimum) {
      emit(ctx, path, `must be >= ${schema.minimum}`)
    }
    if (typeof schema.maximum === 'number' && v > schema.maximum) {
      emit(ctx, path, `must be <= ${schema.maximum}`)
    }
    if (
      typeof schema.exclusiveMinimum === 'number' &&
      v <= schema.exclusiveMinimum
    ) {
      emit(ctx, path, `must be > ${schema.exclusiveMinimum}`)
    }
    if (
      typeof schema.exclusiveMaximum === 'number' &&
      v >= schema.exclusiveMaximum
    ) {
      emit(ctx, path, `must be < ${schema.exclusiveMaximum}`)
    }
  }

  if (typeof v === 'string') {
    if (typeof schema.minLength === 'number' && v.length < schema.minLength) {
      emit(ctx, path, `must be at least ${schema.minLength} character(s)`)
    }
    if (typeof schema.maxLength === 'number' && v.length > schema.maxLength) {
      emit(ctx, path, `must be at most ${schema.maxLength} character(s)`)
    }
    if (typeof schema.pattern === 'string') {
      try {
        if (!new RegExp(schema.pattern).test(v)) {
          emit(ctx, path, `must match pattern ${schema.pattern}`)
        }
      } catch {
        // Invalid regex in the schema — let the server be the authority.
      }
    }
  }
}
