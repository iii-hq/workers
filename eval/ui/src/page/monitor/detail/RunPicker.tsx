// One side of the attach dialog: a searchable picker over the E2E runs and
// the lookup line under it.
import { Selector, type SelectorGroup, uiClasses } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'

export interface RunPickerProps {
  id: string
  label: string
  /** The quiet words after the label: what this side is. */
  hint: string
  value: string
  onChange: (next: string) => void
  groups: SelectorGroup[]
  loading: boolean
  /** Shown when nothing is picked: `Choose a run…`, or why there is nothing to choose. */
  placeholder: string
  disabled: boolean
  invalid: boolean
  /** The status line under the picker; `null` shows nothing. */
  status: ReactNode
  statusId: string
}

export function RunPicker({
  id,
  label,
  hint,
  value,
  onChange,
  groups,
  loading,
  placeholder,
  disabled,
  invalid,
  status,
  statusId,
}: RunPickerProps) {
  const hintId = `${id}-hint`
  return (
    <div className={uiClasses.field}>
      <div className="eval-ui-val-fieldhead">
        <label className={uiClasses.fieldLabel} htmlFor={id}>
          {label}
        </label>
        <span id={hintId} className="eval-ui-val-quiet eval-ui-val-small">
          {hint}
        </span>
      </div>
      <Selector
        id={id}
        aria-label={label}
        // What the label does not say: which side this is, what the lookup found.
        aria-describedby={[hintId, status ? statusId : undefined].filter(Boolean).join(' ')}
        value={value || undefined}
        groups={groups}
        onChange={onChange}
        loading={loading}
        loadingMessage="Loading runs…"
        placeholder={placeholder}
        searchPlaceholder="Search runs…"
        emptyMessage="No run matches"
        disabled={disabled}
        invalid={invalid}
        contentClassName="eval-ui-val-runlist"
      />
      {status ? <div id={statusId}>{status}</div> : null}
    </div>
  )
}
