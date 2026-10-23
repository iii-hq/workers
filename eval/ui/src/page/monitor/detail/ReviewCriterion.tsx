// The success criterion of a suggestion: what counts as success, written down
// before any run exists. The form is shared with "Validate in E2E", which
// registers it when the person presses Start.
import { Button, CardHighlight, Input, SegmentedControl, Select, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { CircleAlert, LoaderCircle, Lock, TriangleAlert } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import type { EvalApi } from '../../../api'
import { formatStamp } from '../../../model'
import type { Direction, E2eCatalogScenario, SuggestionReview, ValidationLink } from '../../../types'
import { Pill } from './marks'
import {
  type CriterionDraft,
  criterionFrozen,
  defaultDraft,
  draftOf,
  effectPhrase,
  effectUnit,
  metricChoices,
  metricName,
  parseCriterion,
  registeredLate,
  scenarioChoices,
  withChoice,
} from './review-model'
import { FieldLine, FormField } from './review-parts'
import { formatClock } from './validation-view'

const DIRECTIONS: Array<{ value: Direction; label: string; icon: false }> = [
  { value: 'decrease', label: 'Lower is better', icon: false },
  { value: 'increase', label: 'Higher is better', icon: false },
]

/** The five fields; `scenarios` undefined hides the scenario (the caller already knows it). */
export function CriterionForm({
  idPrefix,
  draft,
  onChange,
  patterns,
  scenarios,
  catalog = [],
  fromPlan = true,
  disabled,
  showErrors,
}: {
  idPrefix: string
  draft: CriterionDraft
  onChange: (next: CriterionDraft) => void
  patterns: string[]
  scenarios?: string[]
  /** The E2E catalog, offered when the plan names no scenario. */
  catalog?: E2eCatalogScenario[]
  /** The plan names a scenario (the hint says so); otherwise the catalog is offered. */
  fromPlan?: boolean
  disabled: boolean
  /** Errors appear once the person acted, not on an untouched form. */
  showErrors: boolean
}) {
  const parsed = parseCriterion(draft)
  const errors = !parsed.ok && showErrors ? parsed.errors : {}
  const set = (change: Partial<CriterionDraft>) => onChange({ ...draft, ...change })
  const wrong = (text: string | undefined) =>
    text ? (
      <FieldLine tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
        {text}
      </FieldLine>
    ) : null
  return (
    <>
      <FormField
        id={`${idPrefix}-metric`}
        label="Metric"
        help={
          patterns.length > 0
            ? "Signals of the rules the suggestion cites, counted by the monitor in each run's transcript. Or pick an E2E measure."
            : 'The suggestion cites no signal, so the criterion measures an E2E value.'
        }
      >
        <Select
          id={`${idPrefix}-metric`}
          value={draft.choice}
          onChange={(choice) => onChange(withChoice(draft, choice))}
          options={metricChoices(patterns)}
          disabled={disabled}
        />
      </FormField>
      <FormField label="Direction">
        <SegmentedControl
          aria-label="Direction"
          variant="radio"
          value={draft.direction}
          onChange={(direction) => set({ direction })}
          options={DIRECTIONS}
        />
      </FormField>
      <div className="eval-ui-rv-pair">
        <FormField
          id={`${idPrefix}-effect`}
          label="Minimum effect"
          hint={effectUnit(draft.choice)}
          help={`The smallest difference between the two means that counts, in ${effectUnit(draft.choice)}.`}
          line={wrong(errors.effect)}
        >
          <Input
            id={`${idPrefix}-effect`}
            value={draft.effect}
            onChange={(effect) => set({ effect })}
            inputMode="decimal"
            disabled={disabled}
            aria-invalid={errors.effect ? true : undefined}
          />
        </FormField>
        <FormField id={`${idPrefix}-runs`} label="Minimum runs per side" line={wrong(errors.runs)}>
          <Input
            id={`${idPrefix}-runs`}
            value={draft.runs}
            onChange={(runs) => set({ runs })}
            inputMode="numeric"
            disabled={disabled}
            aria-invalid={errors.runs ? true : undefined}
          />
        </FormField>
      </div>
      {scenarios ? (
        <FormField
          id={`${idPrefix}-scenario`}
          label="Scenario"
          hint={
            fromPlan
              ? 'from the plan'
              : catalog.length > 0
                ? 'the plan names none: pick one from the E2E catalog'
                : 'the plan names none'
          }
          line={wrong(errors.scenario)}
        >
          {scenarios.length > 0 || catalog.length > 0 ? (
            <Select
              id={`${idPrefix}-scenario`}
              value={draft.scenario || undefined}
              onChange={(scenario) => set({ scenario })}
              options={[
                ...scenarios
                  .filter((value) => !catalog.some((row) => row.id === value))
                  .map((value) => ({ value, label: value })),
                ...catalog.map((row) => ({ value: row.id, label: row.id, description: row.title || row.summary })),
              ]}
              placeholder="Choose a scenario"
              disabled={disabled}
            />
          ) : (
            <Input
              id={`${idPrefix}-scenario`}
              value={draft.scenario}
              onChange={(scenario) => set({ scenario })}
              placeholder="A harness-e2e scenario id"
              disabled={disabled}
              aria-invalid={errors.scenario ? true : undefined}
            />
          )}
        </FormField>
      ) : null}
    </>
  )
}

function Rows({ rows }: { rows: Array<[string, string]> }) {
  return (
    <dl className="eval-ui-ad-fields">
      {rows.map(([label, value]) => (
        <div key={label} className="eval-ui-ad-field">
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  )
}

export function CriterionBlock({
  api,
  evaluationId,
  review,
  links,
  planScenario,
  narrow,
  onSaved,
  onError,
}: {
  api: EvalApi
  evaluationId: string
  review: SuggestionReview
  links: ValidationLink[]
  planScenario: string | null
  narrow: boolean
  onSaved: (row: SuggestionReview) => void
  onError: (message: string) => void
}) {
  const uid = useId()
  const criterion = review.criterion
  const frozen = criterionFrozen(review, links)
  const first = registeredLate(review, links)
  const scenarios = scenarioChoices(review, planScenario)
  const [editing, setEditing] = useState(false)
  const [catalog, setCatalog] = useState<E2eCatalogScenario[]>([])
  // Only a plan with no scenario needs the catalog; a failed read keeps the free-text field.
  useEffect(() => {
    if (!editing || planScenario) return
    let alive = true
    api
      .e2eScenarios()
      .then((rows) => alive && setCatalog(rows))
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [api, editing, planScenario])
  const [draft, setDraft] = useState<CriterionDraft>(() =>
    criterion ? draftOf(criterion) : defaultDraft(review.patterns, scenarios[0]),
  )
  const [touched, setTouched] = useState(false)
  const [saving, setSaving] = useState(false)
  const parsed = parseCriterion(draft)
  const size = narrow ? 'lg' : 'sm'

  const open = () => {
    setDraft(criterion ? draftOf(criterion) : defaultDraft(review.patterns, scenarios[0]))
    setTouched(false)
    setEditing(true)
  }

  const save = async () => {
    setTouched(true)
    if (!parsed.ok || saving) return
    setSaving(true)
    try {
      const row = await api.review(evaluationId, review.suggestion_index, {
        action: 'set_criterion',
        criterion: parsed.input,
        scenario_id: parsed.scenarioId,
      })
      setEditing(false)
      onSaved(row)
    } catch (cause) {
      onError(errorMessage(cause))
    } finally {
      setSaving(false)
    }
  }

  const head = (
    <div className="eval-ui-ad-plan-head">
      <h4 className="eval-ui-ad-plan-title">Success criterion</h4>
      {editing ? (
        <Pill>Draft</Pill>
      ) : !criterion ? (
        <Pill>Not registered</Pill>
      ) : first !== undefined ? (
        <Pill tone="warn">Registered after the runs</Pill>
      ) : (
        <Pill tone="ok">Registered</Pill>
      )}
    </div>
  )

  if (editing) {
    return (
      <CardHighlight className="eval-ui-ad-plan eval-ui-rv-criterion" role="group" aria-label="Success criterion">
        {head}
        <form
          className="eval-ui-val-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault()
            void save()
          }}
        >
          <div className="eval-ui-val-fields">
            <CriterionForm
              idPrefix={uid}
              draft={draft}
              onChange={(next) => {
                setDraft(next)
                setTouched(true)
              }}
              patterns={review.patterns}
              scenarios={scenarios}
              catalog={catalog}
              fromPlan={Boolean(planScenario)}
              disabled={saving}
              showErrors={touched}
            />
          </div>
          <div className="eval-ui-val-actions" data-stacked={narrow || undefined}>
            <Button type="button" variant="ghost" size={size} disabled={saving} onClick={() => setEditing(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="primary"
              size={size}
              disabled={saving || !parsed.ok}
              aria-busy={saving || undefined}
            >
              {saving ? (
                <>
                  <LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
                  Registering…
                </>
              ) : (
                <>
                  <Lock size={16} aria-hidden="true" />
                  Register criterion
                </>
              )}
            </Button>
          </div>
          <p className="eval-ui-val-note">
            Registering stamps the time. It can be changed until the first run is attached, never after.
          </p>
        </form>
      </CardHighlight>
    )
  }

  if (!criterion) {
    return (
      <CardHighlight className="eval-ui-ad-plan eval-ui-rv-criterion" role="group" aria-label="Success criterion">
        {head}
        <p className="eval-ui-ad-quiet">
          Write down what counts as success before any run. “Validated improvement” is only available when the criterion
          was registered before the first run.
        </p>
        <div>
          <Button variant="pill" size={size} onClick={open}>
            <Lock size={16} aria-hidden="true" />
            Register criterion
          </Button>
        </div>
      </CardHighlight>
    )
  }

  return (
    <CardHighlight className="eval-ui-ad-plan eval-ui-rv-criterion" role="group" aria-label="Success criterion">
      {head}
      <Rows
        rows={[
          [
            'Metric',
            criterion.pattern ? `${criterion.pattern.replace(':', ' · ')} · signals per run` : metricName(criterion),
          ],
          ['Direction', criterion.direction === 'decrease' ? 'Lower is better' : 'Higher is better'],
          ['Minimum effect', `At least ${effectPhrase(criterion)}`],
          ['Minimum runs', `${criterion.min_runs} per side`],
          ['Scenario', criterion.scenario_id],
        ]}
      />
      {first !== undefined ? (
        <StatusPanel
          variant="warn"
          role="status"
          icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
          headline={`Registered ${formatClock(criterion.registered_at)} by ${criterion.registered_by}, after the first run (${formatClock(first)})`}
          detail="It can't prove an improvement. Record No improvement, Regression or Inconclusive."
        />
      ) : (
        <FieldLine tone="ok" icon={<Lock className={uiClasses.icon} aria-hidden />}>
          Registered {formatStamp(criterion.registered_at)} by {criterion.registered_by}
          {frozen
            ? ', before any run. Locked once a run is attached.'
            : '. It can be changed until the first run is attached.'}
        </FieldLine>
      )}
      {frozen ? null : (
        <div>
          <Button variant="ghost" size={size} onClick={open}>
            Edit criterion
          </Button>
        </div>
      )}
    </CardHighlight>
  )
}
