// Pieces the attach dialog and the "E2E runs" panel share: the identity-check
// list, the values that differ, and the Harness-version line.
import { uiClasses } from '@iii-dev/console-ui'
import { Check, X } from 'lucide-react'
import type { ComparabilityCheck, E2eExecution } from '../../../types'
import { checkLabel, checkValue, harnessNote } from './validation-view'

function FieldName({ field }: { field: string }) {
  const label = checkLabel(field)
  return <span className={label.mono ? 'eval-ui-val-mono' : undefined}>{label.text}</span>
}

/** Identity checks as a checklist; `values` adds what both runs hold. */
export function CheckList({ checks, values }: { checks: ComparabilityCheck[]; values: boolean }) {
  return (
    <ul className="eval-ui-val-checks">
      {checks.map((check) => (
        <li key={check.field} className="eval-ui-val-check" data-match={check.matches}>
          {check.matches ? (
            <Check className={uiClasses.icon} aria-hidden />
          ) : (
            <X className={uiClasses.icon} aria-hidden />
          )}
          <FieldName field={check.field} />
          <span className="eval-ui-val-sr">{check.matches ? 'matches' : 'differs'}</span>
          {values ? (
            <span className="eval-ui-val-mono eval-ui-val-quiet">{checkValue(check.field, check.baseline)}</span>
          ) : null}
        </li>
      ))}
    </ul>
  )
}

/** The checks that differ, with both values side by side. */
export function Mismatches({ checks }: { checks: ComparabilityCheck[] }) {
  return (
    <ul className="eval-ui-val-diffs" aria-label="Values that differ">
      {checks.map((check) => (
        <li key={check.field} className="eval-ui-val-diff">
          <span className="eval-ui-val-diff-field">
            <FieldName field={check.field} />
          </span>
          <span className="eval-ui-val-diff-side">
            <span className="eval-ui-val-diff-who">Baseline</span>
            <span className="eval-ui-val-mono">{checkValue(check.field, check.baseline)}</span>
          </span>
          <span className="eval-ui-val-diff-side" data-side="candidate">
            <span className="eval-ui-val-diff-who">Candidate</span>
            <span className="eval-ui-val-mono">{checkValue(check.field, check.candidate)}</span>
          </span>
        </li>
      ))}
    </ul>
  )
}

/** The one value meant to differ between the runs. `short` is the dialog's wording. */
export function HarnessLine({
  baseline,
  candidate,
  short,
}: {
  baseline: E2eExecution
  candidate: E2eExecution
  short?: boolean
}) {
  const note = harnessNote(baseline, candidate)
  if (note.kind === 'differs') {
    return short ? (
      <>Only the Harness version differs.</>
    ) : (
      <>
        Harness version differs, as intended:{' '}
        <span className="eval-ui-val-mono eval-ui-val-ink">
          {note.baseline} → {note.candidate}
        </span>
      </>
    )
  }
  if (note.kind === 'same') {
    return (
      <>
        Both runs report the same Harness version:{' '}
        <span className="eval-ui-val-mono eval-ui-val-ink">{note.version}</span>
      </>
    )
  }
  return <>Harness version not reported.</>
}
