import { useRef } from 'react'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { cn } from '@/lib/utils'
import type { JsonValue } from '../api'
import { FieldDispatch, type FieldProps } from './FieldDispatch'
import { FieldShell } from './FieldShell'
import { pathToDomId } from './path'
import { schemaDefault, withoutNull } from './ref-resolver'

type Mode = 'set' | 'unset'

const MODE_OPTIONS: { value: Mode; label: string }[] = [
  { value: 'set', label: 'set' },
  { value: 'unset', label: 'unset' },
]

/**
 * Wrapper for schemas declared as `type: ["X", "null"]`. Renders a small
 * set/unset toggle on top of the inner field; when "unset" is selected,
 * the saved value is forced to `null` and the inner field disappears so
 * the operator isn't editing a control whose value is being ignored.
 *
 * Flipping to "unset" does NOT discard what was typed: we stash the last
 * set value in a ref and restore it when the operator flips back to "set",
 * so toggling the mode is non-destructive. The first "set" (nothing
 * stashed) seeds the schema default — for a provider `system_prompt`
 * that's the provider-declared prompt, giving the operator an editable
 * starting point instead of a blank box.
 */
export function NullableField(props: FieldProps) {
  const { label, schema, value, onChange, required } = props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const innerSchema = withoutNull(schema)
  const mode: Mode = value === null || value === undefined ? 'unset' : 'set'

  // Remember the last non-null value so unset→set round-trips the draft.
  // Kept current on every render while set, so it captures live edits.
  const lastSetRef = useRef<JsonValue | undefined>(undefined)
  if (mode === 'set') lastSetRef.current = value

  function handleModeChange(next: Mode) {
    if (next === 'unset') {
      onChange(null)
    } else {
      // Restore the stashed draft; first time (nothing stashed) fall back
      // to the schema default (the provider-declared prompt for a slice's
      // system_prompt) or a type-appropriate zero.
      onChange(lastSetRef.current ?? schemaDefault(innerSchema))
    }
  }

  return (
    <div className="space-y-2">
      <FieldShell
        label={label}
        description={description}
        required={required}
        errorMessage={null}
        anchorId={pathToDomId(props.path)}
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
