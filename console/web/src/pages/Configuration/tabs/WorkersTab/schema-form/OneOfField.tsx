import { Select } from '@/components/ui/Select'
import { cn } from '@/lib/utils'
import type { JsonSchema, JsonValue } from '../api'
import { wt } from '../typography'
import { EnumField } from './EnumField'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import { resolveSchema, schemaDefault, schemaTypes } from './ref-resolver'

/**
 * `oneOf` / `anyOf` dispatcher.
 *
 * Two render strategies, picked from the variant shape:
 *
 * 1. **Single-value enum variants** — every variant is `{ enum: [X] }`
 *    (schemars's representation of a Rust enum with `serde(rename_all)`).
 *    We collapse these into a plain `enum` shape and hand off to
 *    `EnumField` so the operator gets a single dropdown instead of a
 *    misleading two-step variant + value picker.
 *
 * 2. **Heterogeneous variants** — different types or shapes. We render
 *    a variant `Select` followed by a recursive `FieldDispatch` for the
 *    chosen sub-schema. Best-effort match-on-load picks the variant
 *    whose `type` matches the current value; ties fall back to the
 *    first variant so the operator can always change it.
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

  // Strategy 2 — heterogeneous variants.
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
