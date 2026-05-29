import { ModeToggle } from '@/components/ui/ModeToggle'
import { wt } from '../typography'
import type { FieldProps } from './FieldDispatch'
import { errorForField, FieldShell } from './FieldShell'

type BoolKey = 'true' | 'false'

const BOOL_OPTIONS: { value: BoolKey; label: string }[] = [
  { value: 'true', label: 'true' },
  { value: 'false', label: 'false' },
]

/**
 * Two-state segmented toggle for `type: boolean`. We reuse `ModeToggle`
 * in `radio` mode so the visual matches the theme picker on the console
 * settings tab — boolean preferences read consistently across the whole
 * page instead of inventing a one-off toggle widget.
 */
export function BooleanField(props: FieldProps) {
  const { label, schema, value, onChange, required, hideLabel } = props
  const description =
    typeof schema.description === 'string' ? schema.description : undefined
  const current: BoolKey = value === true ? 'true' : 'false'

  return (
    <FieldShell
      label={label}
      description={description}
      required={required}
      errorMessage={errorForField(props)}
      hideLabel={hideLabel}
    >
      {() => (
        <ModeToggle<BoolKey>
          value={current}
          onChange={(next) => onChange(next === 'true')}
          options={BOOL_OPTIONS}
          variant="radio"
          aria-label={label}
          className={wt.toggle}
        />
      )}
    </FieldShell>
  )
}
