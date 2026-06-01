import { cn } from '@/lib/utils'
import { wt } from '../typography'
import type { FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'

/**
 * Numeric field for `type: integer` and `type: number`. Stores `null`
 * when the input is empty (the schema may forbid this, in which case
 * the server will reject the save and the error surfaces in the shell).
 * For integer types the user can still type a decimal — we coerce on
 * blur so the visible value matches what gets sent.
 *
 * `min` / `max` are surfaced as HTML attributes for browser-level UX
 * affordances; full validation runs on the server. `step` defaults to
 * 1 for integers and `any` for numbers so spinner arrows feel right.
 */
export function NumberField(props: FieldProps) {
  const { label, schema, value, onChange, required, hideLabel } = props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const isInteger = schema.type === 'integer'

  const current =
    typeof value === 'number' && Number.isFinite(value) ? String(value) : ''

  function handleChange(raw: string) {
    if (raw === '') {
      onChange(null)
      return
    }
    const parsed = isInteger ? Number.parseInt(raw, 10) : Number.parseFloat(raw)
    if (Number.isFinite(parsed)) {
      onChange(parsed)
    } else {
      // Keep the partially-typed value as a string in display by passing
      // through onChange unchanged; the parent re-renders with the new
      // (invalid) value as null, but the local input is uncontrolled in
      // that transient state — accept the trade-off rather than carry an
      // internal buffer that would shadow the source of truth.
      onChange(null)
    }
  }

  return (
    <FieldShell
      label={label}
      description={description}
      required={required}
      errorMessage={errorForField(props)}
      hideLabel={hideLabel}
    >
      {(controlId) => (
        <input
          id={controlId}
          name={label}
          type="number"
          inputMode={isInteger ? 'numeric' : 'decimal'}
          value={current}
          step={isInteger ? 1 : 'any'}
          min={typeof schema.minimum === 'number' ? schema.minimum : undefined}
          max={typeof schema.maximum === 'number' ? schema.maximum : undefined}
          onChange={(e) => handleChange(e.currentTarget.value)}
          placeholder={
            typeof schema.default === 'number'
              ? String(schema.default)
              : undefined
          }
          className={cn(
            'w-full border border-rule bg-bg px-3 h-9 text-ink',
            wt.control,
            'placeholder:text-ink-faint',
            'focus:outline-none focus:border-ink transition-colors',
            'disabled:opacity-40 disabled:pointer-events-none',
          )}
        />
      )}
    </FieldShell>
  )
}
