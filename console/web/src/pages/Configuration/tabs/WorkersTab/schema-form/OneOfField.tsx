import { Select } from '@/components/ui/Select'
import { cn } from '@/lib/utils'
import type { JsonSchema, JsonValue } from '../api'
import { wt } from '../typography'
import { EnumField } from './EnumField'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import {
  discriminator,
  hasRenderableProps,
  isPlainObject,
  mergeNullable,
  nullableUnionInner,
  stripProperty,
} from './oneof-shape'
import { resolveSchema, schemaDefault, schemaTypes } from './ref-resolver'

/**
 * `oneOf` / `anyOf` dispatcher.
 *
 * Render strategies, picked from the variant shape (first match wins):
 *
 * 0. **Nullable union** — exactly `[X, null]` (schemars's encoding of
 *    `Option<T>` for a referenced/typed `T`, e.g. `anyOf: [{ $ref }, {
 *    type: null }]`). We collapse it to a single nullable schema and hand
 *    back to `FieldDispatch`, so an optional enum renders as one select
 *    with an "unset" entry and an optional object/string gets a set/unset
 *    toggle — instead of a misleading two-step "X | null" picker.
 *
 * 1. **Single-value enum variants** — every variant is `{ enum: [X] }`
 *    (schemars's representation of a Rust enum with `serde(rename_all)`).
 *    Collapsed into a plain `enum` and handed to `EnumField`.
 *
 * 2. **Discriminated object union** — every variant is an object pinning a
 *    shared key to a distinct single-value `enum`/`const` (schemars's
 *    adjacently-tagged enum: `{ name: { enum: ["kv"] }, config: … }`). We
 *    render one dropdown for the discriminator plus a sub-form for the rest
 *    of the chosen branch. Matching the active branch by the discriminator
 *    value (not by JSON type) is what makes switching branches stick.
 *
 * 3. **Heterogeneous variants** — different types or shapes. Variant
 *    `Select` + recursive `FieldDispatch` for the chosen sub-schema, with a
 *    best-effort match-on-load that falls back to the first variant.
 */
export function OneOfField(props: FieldProps) {
  const { label, schema, value, onChange, required, rootSchema } = props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const rawVariants = Array.isArray(schema.oneOf)
    ? (schema.oneOf as JsonSchema[])
    : Array.isArray(schema.anyOf)
      ? (schema.anyOf as JsonSchema[])
      : []
  const variants = rawVariants.map((v) =>
    resolveSchema(v, { root: rootSchema }),
  )

  // Strategy 0 — `[X, null]` nullable union → a single nullable schema.
  const nullableInner = nullableUnionInner(variants)
  if (nullableInner) {
    return (
      <FieldDispatch {...props} schema={mergeNullable(schema, nullableInner)} />
    )
  }

  // Strategy 1 — collapse a string-enum union into a flat enum.
  if (variants.length > 0 && variants.every(isSingleStringEnumVariant)) {
    const flatEnum: JsonValue[] = variants.map(
      (v) => (v.enum as JsonValue[])[0],
    )
    const flatNames = variants.map((v) =>
      typeof v.title === 'string'
        ? v.title
        : typeof v.description === 'string'
          ? v.description
          : String((v.enum as JsonValue[])[0]),
    )
    const synthetic: JsonSchema = {
      ...schema,
      enum: flatEnum,
      enumNames: flatNames,
    }
    return <EnumField {...props} schema={synthetic} />
  }

  // Strategy 2 — discriminated object union.
  const disc = discriminator(variants)
  if (disc) {
    const current = isPlainObject(value)
      ? (value as Record<string, JsonValue>)
      : undefined
    const currentDisc =
      current && typeof current[disc.key] === 'string'
        ? (current[disc.key] as string)
        : undefined
    const activeIdx = disc.values.findIndex((v) => v === currentDisc)
    const options = variants.map((v, idx) => ({
      value: disc.values[idx],
      label: typeof v.title === 'string' ? v.title : disc.values[idx],
    }))
    const activeVariant = activeIdx >= 0 ? variants[activeIdx] : undefined
    const subSchema = activeVariant
      ? stripProperty(activeVariant, disc.key)
      : undefined

    return (
      <div className="space-y-2">
        <FieldShell
          label={label}
          description={description}
          required={required}
          errorMessage={errorForField(props)}
        >
          {() => (
            <Select<string>
              value={currentDisc}
              // Reset to a fresh branch carrying only the discriminator; the
              // previous branch's config does not apply to the new one.
              onChange={(next) =>
                onChange({ [disc.key]: next } as JsonValue)
              }
              options={options}
              aria-label={label}
              placeholder="select…"
              className={cn('w-full', wt.control)}
            />
          )}
        </FieldShell>
        {subSchema && hasRenderableProps(subSchema) ? (
          <div className={cn('pl-3 border-l border-rule')}>
            <FieldDispatch {...props} schema={subSchema} hideLabel />
          </div>
        ) : null}
      </div>
    )
  }

  // Strategy 3 — heterogeneous variants.
  const activeIdx = matchVariantIndex(variants, value)
  const options = variants.map((v, idx) => ({
    value: String(idx),
    label: variantLabel(v, idx),
  }))

  function handleVariantChange(nextKey: string) {
    const nextIdx = Number.parseInt(nextKey, 10)
    if (!Number.isFinite(nextIdx)) return
    onChange(schemaDefault(variants[nextIdx]))
  }

  return (
    <div className="space-y-2">
      <FieldShell
        label={label}
        description={description}
        required={required}
        errorMessage={errorForField(props)}
      >
        {() => (
          <Select<string>
            value={String(activeIdx)}
            onChange={handleVariantChange}
            options={options}
            aria-label={`${label} variant`}
            className={cn('w-full', wt.control)}
          />
        )}
      </FieldShell>
      <div className={cn('pl-3 border-l border-rule')}>
        <FieldDispatch
          {...props}
          label={`${label} value`}
          schema={variants[activeIdx]}
        />
      </div>
    </div>
  )
}

function isSingleStringEnumVariant(variant: JsonSchema): boolean {
  if (!Array.isArray(variant.enum) || variant.enum.length !== 1) return false
  const types = schemaTypes(variant)
  // Accept either explicit `type: string` or an unspecified type with a
  // string-typed enum value (some schemars outputs omit `type`).
  if (types.length > 0 && !types.includes('string')) return false
  return typeof (variant.enum as unknown[])[0] === 'string'
}

function variantLabel(variant: JsonSchema, idx: number): string {
  if (typeof variant.title === 'string') return variant.title
  if (Array.isArray(variant.enum) && variant.enum.length === 1) {
    const single = (variant.enum as unknown[])[0]
    if (typeof single === 'string') return single
  }
  const types = schemaTypes(variant)
  if (types.length > 0) return types.join(' | ')
  return `variant ${idx + 1}`
}

/**
 * Best-effort match of the current value to one of the variant schemas.
 * Returns the index of the first matching variant, or 0 when no variant
 * fits (so the dispatcher always has something to render).
 */
function matchVariantIndex(
  variants: JsonSchema[],
  value: JsonValue | undefined,
): number {
  for (let i = 0; i < variants.length; i++) {
    if (valueMatchesSchema(value, variants[i])) return i
  }
  return 0
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
