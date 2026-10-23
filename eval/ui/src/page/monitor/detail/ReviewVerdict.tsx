// The person's verdict on a validation: four outcomes, the reason, and the
// controls they checked. The code proposes an outcome next to the numbers, and
// "validated improvement" is refused, with the reason on the option, unless
// the criterion came first and the runs match it. The monitor never decides.
import { Button, Checkbox, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { CircleAlert, LoaderCircle, TriangleAlert } from 'lucide-react'
import { useId, useState } from 'react'
import type { EvalApi } from '../../../api'
import { formatStamp } from '../../../model'
import type { Evidence, SuggestionReview, ValidationLink, ValidationOutcome, ValidationPlan } from '../../../types'
import { Pill } from './marks'
import {
  disagrees,
  improvementRefusal,
  OUTCOME_HINT,
  OUTCOME_LABEL,
  OUTCOME_TONE,
  OUTCOMES,
  PROPOSAL_LABEL,
  sameVerdict,
  verdictControls,
} from './review-model'
import { DialogFrame, FieldLine, FormActions, FormField, TextArea } from './review-parts'

/** What the code computed from the criterion and the counted runs: a proposal, never the verdict. */
export function Proposal({ evidence, note }: { evidence: Evidence; note: string }) {
  return (
    <div className="eval-ui-rv-proposal">
      <div className="eval-ui-rv-proposal-head">
        <span className="eval-ui-val-label">Code proposes</span>
        <Pill tone={OUTCOME_TONE[evidence.computed_outcome]}>{PROPOSAL_LABEL[evidence.computed_outcome]}</Pill>
      </div>
      <p className="eval-ui-rv-proposal-reason">{evidence.reason}</p>
      <p className="eval-ui-val-note">{note}</p>
    </div>
  )
}

function VerdictForm({
  api,
  evaluationId,
  review,
  links,
  plan,
  sheet,
  narrow,
  onSaved,
  onCancel,
}: {
  api: EvalApi
  evaluationId: string
  review: SuggestionReview
  links: ValidationLink[]
  plan: ValidationPlan
  sheet: boolean
  narrow: boolean
  onSaved: (row: SuggestionReview) => void
  onCancel: () => void
}) {
  const uid = useId()
  const controls = verdictControls(plan)
  const [outcome, setOutcome] = useState<ValidationOutcome | null>(review.verdict?.outcome ?? null)
  const [rationale, setRationale] = useState(review.verdict?.rationale ?? '')
  const [checked, setChecked] = useState<string[]>(
    (review.verdict?.controls_checked ?? []).filter((control) => controls.includes(control)),
  )
  const [saving, setSaving] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const refusal = improvementRefusal(review, links)
  const current = review.verdict
  const unchanged = sameVerdict(current, { outcome, rationale, checked }, controls)
  const ready =
    outcome !== null && rationale.trim() !== '' && !(outcome === 'validated_improvement' && refusal) && !unchanged
  const size = narrow ? 'lg' : 'sm'

  const submit = async () => {
    if (!ready || saving || outcome === null) return
    setSaving(true)
    setFailure(null)
    try {
      onSaved(
        await api.review(evaluationId, review.suggestion_index, {
          action: 'set_verdict',
          outcome,
          rationale: rationale.trim(),
          // In the order they are listed, whatever order they were ticked in.
          controls_checked: controls.filter((control) => checked.includes(control)),
        }),
      )
    } catch (cause) {
      setFailure(errorMessage(cause))
      setSaving(false)
    }
  }

  return (
    <form
      className="eval-ui-val-form"
      data-narrow={narrow || undefined}
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      <div className="eval-ui-val-fields">
        {review.evidence ? (
          <Proposal
            evidence={review.evidence}
            note={
              current
                ? 'Your current verdict is filled in below: change it and record, or cancel.'
                : 'Nothing below is pre-selected.'
            }
          />
        ) : null}
        <fieldset className="eval-ui-rv-options" disabled={saving}>
          <legend className="eval-ui-val-sr">Outcome</legend>
          {OUTCOMES.map((value) => {
            const refused = value === 'validated_improvement' ? refusal : undefined
            return (
              <label
                key={value}
                className="eval-ui-rv-option"
                data-checked={outcome === value || undefined}
                data-disabled={refused ? true : undefined}
              >
                <input
                  type="radio"
                  name={`${uid}-outcome`}
                  value={value}
                  checked={outcome === value}
                  disabled={refused !== undefined}
                  onChange={() => setOutcome(value)}
                />
                <span className="eval-ui-rv-option-copy">
                  <span className="eval-ui-rv-option-label">{OUTCOME_LABEL[value]}</span>
                  <span className="eval-ui-val-quiet eval-ui-val-small">{OUTCOME_HINT[value]}</span>
                  {refused ? (
                    <FieldLine tone="warn" icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}>
                      {refused}
                    </FieldLine>
                  ) : null}
                </span>
              </label>
            )
          })}
        </fieldset>
        <FormField id={`${uid}-why`} label="Rationale" hint="required">
          <TextArea
            id={`${uid}-why`}
            value={rationale}
            onChange={setRationale}
            placeholder="What you checked and why you conclude this"
            rows={4}
            maxLength={4000}
            disabled={saving}
          />
        </FormField>
        <fieldset className="eval-ui-rv-controls" disabled={saving}>
          <legend className="eval-ui-val-fieldhead">
            <span className={uiClasses.fieldLabel}>Controls</span>
            <span className="eval-ui-val-quiet eval-ui-val-small">
              from the plan and the E2E · {checked.length} of {controls.length} checked
            </span>
          </legend>
          {controls.map((control) => (
            <Checkbox
              key={control}
              label={control}
              checked={checked.includes(control)}
              onChange={(event) =>
                setChecked((now) => (event.target.checked ? [...now, control] : now.filter((item) => item !== control)))
              }
            />
          ))}
          <p className="eval-ui-val-note">Unchecked controls don't block the verdict. They are recorded with it.</p>
        </fieldset>
        <p className="eval-ui-val-note">A verdict isn't edited; recording another replaces it.</p>
        {failure ? (
          <StatusPanel
            variant="alert"
            role="alert"
            icon={<CircleAlert className={uiClasses.icon} aria-hidden />}
            headline="That did not go through"
            detail={failure}
          />
        ) : null}
      </div>
      <FormActions
        sheet={sheet}
        narrow={narrow}
        cancel={
          <Button type="button" variant={sheet ? 'pill' : 'ghost'} size={size} disabled={saving} onClick={onCancel}>
            Cancel
          </Button>
        }
        submit={
          <Button
            type="submit"
            variant="primary"
            size={size}
            disabled={!ready || saving}
            aria-busy={saving || undefined}
          >
            {saving ? (
              <>
                <LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
                Recording…
              </>
            ) : (
              'Record verdict'
            )}
          </Button>
        }
      />
    </form>
  )
}

export function VerdictDialog(props: {
  api: EvalApi
  evaluationId: string
  review: SuggestionReview
  links: ValidationLink[]
  plan: ValidationPlan
  title: string
  narrow: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  onSaved: (row: SuggestionReview) => void
}) {
  const { open, onOpenChange, title, narrow, onSaved, ...form } = props
  return (
    <DialogFrame
      open={open}
      onOpenChange={onOpenChange}
      title={`Record your verdict on S${form.review.suggestion_index + 1}`}
      description={title}
      narrow={narrow}
    >
      {({ sheet }) => (
        <VerdictForm
          {...form}
          sheet={sheet}
          narrow={narrow}
          onSaved={(row) => {
            onOpenChange(false)
            onSaved(row)
          }}
          onCancel={() => onOpenChange(false)}
        />
      )}
    </DialogFrame>
  )
}

/** "Your verdict" once recorded: the outcome, who and when, the reason, and how it stands against the proposal. */
export function VerdictTile({
  review,
  plan,
  narrow,
  onChange,
}: {
  review: SuggestionReview
  plan: ValidationPlan
  narrow: boolean
  onChange: () => void
}) {
  const verdict = review.verdict
  if (!verdict) return null
  const proposed = review.evidence?.computed_outcome
  return (
    <div className="eval-ui-rv-verdict">
      <div className="eval-ui-rv-proposal-head">
        <span className="eval-ui-val-label">Your verdict</span>
        <Pill tone={OUTCOME_TONE[verdict.outcome]} strong={OUTCOME_TONE[verdict.outcome] === 'neutral'}>
          {OUTCOME_LABEL[verdict.outcome]}
        </Pill>
        <span className="eval-ui-val-mono eval-ui-val-quiet eval-ui-rv-verdict-by">
          {verdict.by} · {formatStamp(verdict.at)}
        </span>
      </div>
      <p className="eval-ui-rv-proposal-reason">{verdict.rationale}</p>
      <p className="eval-ui-val-note">
        {proposed
          ? disagrees(review)
            ? `Code proposed ${PROPOSAL_LABEL[proposed]}; you recorded ${OUTCOME_LABEL[verdict.outcome]}. `
            : `Code proposed ${PROPOSAL_LABEL[proposed]}. `
          : ''}
        {verdict.controls_checked.length} of {verdictControls(plan).length} controls checked.
      </p>
      <div>
        <Button variant="ghost" size={narrow ? 'lg' : 'sm'} onClick={onChange}>
          Change verdict
        </Button>
      </div>
    </div>
  )
}
