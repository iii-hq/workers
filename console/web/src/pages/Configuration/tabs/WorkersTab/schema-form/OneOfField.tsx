import { Select } from '@/components/ui/Select'
import { cn } from '@/lib/utils'
import type { JsonSchema, JsonValue } from '../api'
import { wt } from '../typography'
import { EnumField } from './EnumField'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import { resolveSchema } from './ref-resolver'
import {
  isSingleStringEnumVariant,
  matchVariantIndex,
  variantDefault,
  variantLabel,
} from './variant-match'

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
 *    chosen sub-schema. Match-on-load prefers a discriminated tag (an
 *    adjacently/internally tagged enum's single-value `name`/`type`
 *    property — e.g. an adapter `{ name: "bridge" }`), then falls back to
 *    structural `type` matching, then to the first variant so the
 *    operator can always change it. See [`./variant-match`].
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
    onChange(variantDefault(variants[nextIdx], rootSchema))
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
