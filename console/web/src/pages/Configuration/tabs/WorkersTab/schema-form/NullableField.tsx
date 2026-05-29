import { ModeToggle } from '@/components/ui/ModeToggle'
import { cn } from '@/lib/utils'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { FieldShell } from './FieldShell'
import { schemaDefault, withoutNull } from './ref-resolver'

type Mode = 'set' | 'unset'

const MODE_OPTIONS: { value: Mode; label: string }[] = [
  { value: 'set', label: 'set' },
  { value: 'unset', label: 'unset' },
]

/**
 * Wrapper for schemas declared as `type: ["X", "null"]`. Renders a small
 * set/unset toggle on top of the inner field; when "unset" is selected,
 * the value is forced to `null` and the inner field disappears so the
 * operator isn't editing a control whose value is being ignored.
 *
 * When the operator flips back to "set", we seed the inner field with
 * the schema-provided default (or a type-appropriate zero) so they don't
 * land on `null` and bounce back to the unset state on the next render.
 */
export function NullableField(props: FieldProps) {
  const { label, schema, value, onChange, required } = props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const innerSchema = withoutNull(schema)
  const mode: Mode = value === null || value === undefined ? 'unset' : 'set'

  function handleModeChange(next: Mode) {
    if (next === 'unset') {
      onChange(null)
    } else {
      // Seed with the schema default (or type-appropriate zero) so the
      // inner field has something to render once it appears.
      onChange(schemaDefault(innerSchema))
    }
  }

  return (
    <div className="space-y-2">
      <FieldShell
        label={label}
        description={description}
        required={required}
        errorMessage={null}
      >
        {() => (
          <ModeToggle<Mode>
            value={mode}
            onChange={handleModeChange}
            options={MODE_OPTIONS}
            variant="radio"
            aria-label={`${label} value mode`}
          />
        )}
      </FieldShell>
      {mode === 'set' ? (
        <div className={cn('pl-3 border-l border-rule')}>
          <FieldDispatch
            {...props}
            label={`${label} value`}
            schema={innerSchema}
          />
        </div>
      ) : null}
    </div>
  )
}
