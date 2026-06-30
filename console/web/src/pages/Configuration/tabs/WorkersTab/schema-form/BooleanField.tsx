import { ModeToggle } from '@/components/ui/ModeToggle'
import { wt } from '../typography'
import type { FieldProps } from './FieldDispatch'
import { TemplatableField } from './TemplatableField'

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
 *
 * A `${VAR}` env template can also live in a boolean field; `TemplatableField`
 * swaps in the pill editor when the value is a string and offers a toggle
 * between a literal value and an env template.
 */
export function BooleanField(props: FieldProps) {
  const { label, value, onChange } = props
  const current: BoolKey = value === true ? 'true' : 'false'

  return (
    <TemplatableField
      props={props}
      scalarDefault={false}
      renderScalar={() => (
        <ModeToggle<BoolKey>
          value={current}
          onChange={(next) => onChange(next === 'true')}
          options={BOOL_OPTIONS}
          variant="radio"
          aria-label={label}
          className={wt.toggle}
        />
      )}
    />
  )
}
