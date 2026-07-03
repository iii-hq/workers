import { Select } from '@/components/ui/Select'
import { cn } from '@/lib/utils'
import type { JsonValue } from '../api'
import { wt } from '../typography'
import type { FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'
import { pathToDomId } from './path'

/**
 * Enum field for schemas with an `enum` keyword. Options come straight
 * from the schema; we stringify each enum value so the Radix-based
 * `Select` (which is string-typed) can render it, and re-hydrate the
 * original primitive value on change so booleans and numbers round-trip.
 *
 * The schema's `enumNames` (a common extension) is honored when present
 * so worker authors can give operators human-friendly labels without
 * changing the underlying value.
 */
export function EnumField(props: FieldProps) {
  const { label, schema, value, onChange, required, hideLabel } = props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined

  const rawEnum = Array.isArray(schema.enum) ? (schema.enum as JsonValue[]) : []
  const enumNames =
    Array.isArray(schema.enumNames) &&
    (schema.enumNames as unknown[]).every((n) => typeof n === 'string')
      ? (schema.enumNames as string[])
      : null

  const options = rawEnum.map((raw, idx) => ({
    value: encodeEnumValue(raw),
    label: enumNames?.[idx] ?? defaultLabel(raw),
  }))

  // Leave the key `undefined` when the value is unset so the trigger shows
  // the placeholder instead of stringifying `undefined` into `j:undefined`.
  const currentKey =
    value === undefined ? undefined : encodeEnumValue(value as JsonValue)

  return (
    <FieldShell
      label={label}
      description={description}
      required={required}
      errorMessage={errorForField(props)}
      hideLabel={hideLabel}
      anchorId={pathToDomId(props.path)}
    >
      {() => (
        <Select<string>
          value={currentKey}
          onChange={(nextKey) => {
            const match = rawEnum[options.findIndex((o) => o.value === nextKey)]
            if (match !== undefined) onChange(match)
          }}
          options={options}
          // Optional enums can be cleared back to "unset"; `null` is the
          // form's canonical empty value (matches NumberField / NullableField).
          allowEmpty={!required}
          emptyLabel="unset"
          onClear={() => onChange(null)}
          aria-label={label}
          placeholder="select…"
          className={cn('w-full', wt.control)}
        />
      )}
    </FieldShell>
  )
}

/**
 * Stable string encoding for an arbitrary enum value. We can't use the
 * raw JSON because Radix Select requires string keys; the encoded form
 * is opaque (`json:<stringified>`) so it never collides with a real
 * string enum that happens to equal another value's stringification.
 */
function encodeEnumValue(value: JsonValue): string {
  if (typeof value === 'string') return `s:${value}`
  return `j:${JSON.stringify(value)}`
}

function defaultLabel(value: JsonValue): string {
  if (typeof value === 'string') return value
  return JSON.stringify(value)
}
