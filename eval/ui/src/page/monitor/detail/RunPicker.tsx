// One side of the attach dialog: a searchable picker over the E2E runs, its
// "Jev" mark when Jev filled it, and the lookup line under it.
import { Selector, type SelectorGroup, uiClasses } from '@iii-dev/console-ui'
import { Sparkles } from 'lucide-react'
import type { ReactNode } from 'react'

/** Sits on a picker Jev filled; gone as soon as the user changes that picker. */
export function JevMark({ id }: { id: string }) {
  return (
    <span id={id} className="eval-ui-val-jev">
      <Sparkles className={uiClasses.icon} aria-hidden />
      Jev
      <span className="eval-ui-val-sr"> picked this one</span>
    </span>
  )
}

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
  jev: boolean
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
  jev,
  status,
  statusId,
}: RunPickerProps) {
  const hintId = `${id}-hint`
  const markId = `${id}-jev`
  return (
    <div className={uiClasses.field}>
      <div className="eval-ui-val-fieldhead">
        <label className={uiClasses.fieldLabel} htmlFor={id}>
          {label}
        </label>
        <span id={hintId} className="eval-ui-val-quiet eval-ui-val-small">
          {hint}
        </span>
        {jev ? <JevMark id={markId} /> : null}
      </div>
      <Selector
        id={id}
        aria-label={label}
        // What the label does not say: which side this is, who filled it, what the lookup found.
        aria-describedby={[hintId, jev ? markId : undefined, status ? statusId : undefined].filter(Boolean).join(' ')}
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
