// "Validate S1 in E2E": the one action that spends model money, so everything
// it will do is on this dialog before Start. It registers the criterion if
// none exists, then asks the E2E to run the scenario twice in Docker (the
// Harness without the change, the Harness with it) and shows the cost and time
// the E2E's own past executions suggest. Nothing starts until Start is pressed.
import { Button, Input, Select, Selector, type SelectorGroup, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import {
  CircleAlert,
  CircleCheck,
  Clock,
  FlaskConical,
  LoaderCircle,
  Lock,
  Plug,
  RefreshCw,
  TriangleAlert,
} from 'lucide-react'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import type { EvalApi } from '../../../api'
import type { CatalogModel, E2eCatalogScenario, SuggestionReview, ValidationLink, ValidationPlan } from '../../../types'
import { CriterionForm } from './ReviewCriterion'
import {
  criterionFrozen,
  criterionSummary,
  defaultDraft,
  draftOf,
  MIN_RUNS_FLOOR,
  parseCriterion,
} from './review-model'
import { DialogFrame, FieldLine, FormActions, FormField } from './review-parts'
import {
  checkFor,
  classifyStartFailure,
  costApprox,
  defaultModel,
  dockerHistory,
  type Estimate,
  estimateFor,
  MIN_PAST,
  minutesApprox,
  needsCheck,
  parseRuns,
  REF_CHECK_DELAY_MS,
  type RefAnswer,
  type RefCheck,
  type RefInputs,
  refInputs,
  refKey,
  refusedAnswer,
  resolvedAnswer,
  resolvedLine,
  type StartFailure,
  sentence,
  settleCheck,
  startAllowed,
} from './start-validation-model'
import { formatClock } from './validation-view'

type Loaded<T> = { phase: 'loading' } | { phase: 'ready'; value: T } | { phase: 'failed' }

/** What the dialog reads when it opens: the scenarios, the models and the past executions behind the estimate. */
function useStartData(api: EvalApi) {
  const [scenarios, setScenarios] = useState<Loaded<E2eCatalogScenario[]>>({ phase: 'loading' })
  const [models, setModels] = useState<Loaded<CatalogModel[]>>({ phase: 'loading' })
  const [executions, setExecutions] = useState<Loaded<unknown>>({ phase: 'loading' })
  useEffect(() => {
    let alive = true
    const read = <T,>(work: Promise<T>, set: (next: Loaded<T>) => void) =>
      work.then(
        (value) => alive && set({ phase: 'ready', value }),
        () => alive && set({ phase: 'failed' }),
      )
    void read(api.e2eScenarios(), setScenarios)
    void read(api.models(), setModels)
    void read(api.e2eExecutions(), setExecutions)
    return () => {
      alive = false
    }
  }, [api])
  return { scenarios, models, executions }
}

const modelKey = (provider: string, model: string) => `${provider}\n${model}`

function Message({ failure }: { failure: StartFailure | undefined }) {
  return failure ? (
    <FieldLine tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
      {sentence(failure.text)}
    </FieldLine>
  ) : null
}

/** What the dry run said about one side, under its field. The warnings and the wait are said once, under the baseline and the candidate. */
function RefLine({ check, side }: { check: ReturnType<typeof checkFor>; side: 'candidate' | 'baseline' }) {
  if (!check) return null
  if (check.phase === 'checking') {
    return side === 'candidate' ? (
      <FieldLine role="status" icon={<LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />}>
        Checking the commits…
      </FieldLine>
    ) : null
  }
  if (check.phase === 'refused') {
    return check.field === side ? (
      <FieldLine tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
        {sentence(check.text)}
      </FieldLine>
    ) : null
  }
  return (
    <>
      <FieldLine tone="ok" icon={<CircleCheck className={uiClasses.icon} aria-hidden />}>
        {resolvedLine(check.resolution[side])}
      </FieldLine>
      {side === 'baseline'
        ? check.resolution.warnings.map((warning) => (
            <FieldLine key={warning} tone="warn" icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}>
              {sentence(warning)}
            </FieldLine>
          ))
        : null}
    </>
  )
}

function StartForm({
  api,
  evaluationId,
  review,
  links,
  plan,
  observed,
  sheet,
  narrow,
  onStarted,
  onRefresh,
  onAttachExisting,
  onBusy,
  onCancel,
}: {
  api: EvalApi
  evaluationId: string
  review: SuggestionReview
  links: ValidationLink[]
  plan: ValidationPlan
  /** The model and provider the observed session ran with. */
  observed: { model: string; provider: string } | undefined
  sheet: boolean
  narrow: boolean
  onStarted: (row: SuggestionReview) => void
  /** The start failed after something may have been recorded: read the analysis again. */
  onRefresh: () => void
  onAttachExisting: () => void
  /** Closing is refused while the executions are being started. */
  onBusy: (busy: boolean) => void
  onCancel: () => void
}) {
  const uid = useId()
  const data = useStartData(api)
  const criterion = review.criterion
  const frozen = criterionFrozen(review, links)
  const [scenario, setScenario] = useState(criterion?.scenario_id ?? plan.scenario_id ?? '')
  const [candidate, setCandidate] = useState('')
  const [baseline, setBaseline] = useState<string | null>(null)
  // What the person picked; until then the default follows what Docker has run.
  const [model, setModel] = useState('')
  const [runs, setRuns] = useState('5')
  const [draft, setDraft] = useState(() =>
    criterion ? draftOf(criterion) : defaultDraft(review.patterns, plan.scenario_id ?? undefined),
  )
  const [editing, setEditing] = useState(false)
  const [starting, setStarting] = useState(false)
  const [failure, setFailure] = useState<StartFailure | undefined>()
  useEffect(() => onBusy(starting), [onBusy, starting])

  // Models whose provider Docker has run come first: the stack may not carry the others, and a Docker build would be
  // spent before the executions fail. The observed model's provider (a CLI provider, say) may not be one of them.
  const history = data.executions.phase === 'ready' ? dockerHistory(data.executions.value) : undefined
  const proven = (provider: string) => !history || history.providers.size === 0 || history.providers.has(provider)
  const preferred = defaultModel(observed, history)
  const catalog = data.models.phase === 'ready' ? data.models.value : []
  const choices = catalog.map((item) => ({ model: item.id, provider: item.provider }))
  for (const extra of [preferred, observed]) {
    if (extra && !choices.some((item) => item.model === extra.model && item.provider === extra.provider))
      choices.unshift(extra)
  }
  const options = [
    ...choices.filter((item) => proven(item.provider)),
    ...choices.filter((item) => !proven(item.provider)),
  ].map((item) => ({
    value: modelKey(item.provider, item.model),
    label: `${item.model} · ${item.provider}${proven(item.provider) ? '' : ' · not run in Docker yet'}`,
  }))
  const observedKey = observed ? modelKey(observed.provider, observed.model) : undefined
  const chosen = model || (preferred ? modelKey(preferred.provider, preferred.model) : '') || options[0]?.value || ''
  const [provider = '', modelId = ''] = chosen.split('\n')

  const known = data.scenarios.phase === 'ready' ? data.scenarios.value : []
  const describe = (id: string): string | undefined => {
    const found = known.find((item) => item.id === id)
    return found ? found.summary || found.title || undefined : undefined
  }
  const planOption = plan.scenario_id
    ? [{ value: plan.scenario_id, label: plan.scenario_id, description: describe(plan.scenario_id) }]
    : []
  const groups: SelectorGroup[] = [
    ...(planOption.length > 0 ? [{ label: 'From the plan', options: planOption }] : []),
    {
      label: planOption.length > 0 ? 'Catalog' : 'Scenarios',
      options: known
        .filter((item) => item.id !== plan.scenario_id)
        .map((item) => ({
          value: item.id,
          label: item.id,
          description: item.summary || item.title || undefined,
          keywords: [item.title],
        })),
    },
  ].filter((group) => group.options.length > 0)

  const parsed = parseCriterion({ ...draft, scenario })
  const minimum = parsed.ok ? parsed.input.min_runs : MIN_RUNS_FLOOR
  const runsParsed = parseRuns(runs, minimum)
  const runsError = 'error' in runsParsed ? runsParsed.error : undefined

  // What the backend says about the refs, asked once typing pauses and when a field loses focus. Start waits for an
  // answer to exactly what is in the form now; an answer to what was there before is ignored.
  const runCount = 'runs' in runsParsed ? runsParsed.runs : undefined
  const inputs = useMemo(
    () => refInputs({ scenario, candidate, baseline, runs: runCount }),
    [scenario, candidate, baseline, runCount],
  )
  const key = inputs && refKey(inputs)
  const [check, setCheck] = useState<RefCheck>({ phase: 'idle' })
  const latest = useRef(check)
  const settle = useCallback((answer: RefAnswer) => {
    latest.current = settleCheck(latest.current, answer)
    setCheck(latest.current)
  }, [])
  const ask = useCallback(
    (asked: RefInputs) => {
      const asking = refKey(asked)
      if (!needsCheck(latest.current, asking)) return
      latest.current = { phase: 'checking', key: asking }
      setCheck(latest.current)
      api
        .resolveValidation(evaluationId, review.suggestion_index, {
          scenario_id: asked.scenario,
          candidate_ref: asked.candidate,
          baseline_ref: asked.baseline ?? undefined,
          runs: asked.runs,
        })
        .then(
          (resolution) => settle(resolvedAnswer(asking, resolution)),
          (cause) => settle(refusedAnswer(asking, errorMessage(cause))),
        )
    },
    [api, evaluationId, review.suggestion_index, settle],
  )
  useEffect(() => {
    if (!inputs) return
    const timer = setTimeout(() => ask(inputs), REF_CHECK_DELAY_MS)
    return () => clearTimeout(timer)
  }, [ask, inputs])
  const current = checkFor(check, key)
  const checkRefs = () => {
    if (inputs) ask(inputs)
  }

  const ready = chosen !== '' && parsed.ok && !runsError && startAllowed(check, key)
  const size = narrow ? 'lg' : 'sm'

  const estimate: Estimate | undefined =
    data.executions.phase === 'ready' && 'runs' in runsParsed && scenario && chosen
      ? estimateFor(data.executions.value, { scenarioId: scenario, model: modelId, provider, runs: runsParsed.runs })
      : undefined

  const start = async () => {
    if (!ready || starting || !parsed.ok || !('runs' in runsParsed) || check.phase !== 'resolved') return
    setStarting(true)
    setFailure(undefined)
    try {
      onStarted(
        await api.startValidation(evaluationId, review.suggestion_index, {
          scenario_id: scenario,
          // The commits the person was shown, not the names: a fetch since then must not change what runs.
          candidate_ref: check.resolution.candidate.commit,
          baseline_ref: check.resolution.baseline.commit,
          runs: runsParsed.runs,
          model: modelId,
          provider,
          criterion: parsed.input,
        }),
      )
    } catch (cause) {
      const next = classifyStartFailure(errorMessage(cause))
      setFailure(next)
      setStarting(false)
      // An E2E that failed to answer may have started the baseline: the card shows what was recorded.
      if (next.field === 'e2e') onRefresh()
    }
  }

  const at = (field: StartFailure['field']) => (failure?.field === field ? failure : undefined)
  const criterionLine = criterion
    ? `Criterion registered ${formatClock(criterion.registered_at)} by ${criterion.registered_by}`
    : 'Criterion · registered when you press Start'

  return (
    <form
      className="eval-ui-val-form"
      data-narrow={narrow || undefined}
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void start()
      }}
    >
      <div className="eval-ui-val-fields">
        <p className="eval-ui-val-hint-text">
          Starts two executions in Docker with the same scenario, model and number of runs: the baseline on the Harness
          without the change, the candidate with it. They run in parallel and their runs attach to this suggestion when
          both finish.
        </p>

        {failure && (failure.field === 'e2e' || failure.field === 'other') ? (
          <StatusPanel
            variant="alert"
            role="alert"
            icon={<Plug className={uiClasses.icon} aria-hidden />}
            headline={
              failure.busy
                ? 'The E2E is busy'
                : failure.unconfirmed
                  ? "The E2E didn't answer in time"
                  : failure.field === 'e2e'
                    ? "The E2E service isn't answering"
                    : "Couldn't start the validation"
            }
            detail={
              <>
                {failure.field === 'e2e' && !failure.unconfirmed
                  ? `${failure.busy ? 'Starting needs the E2E free of other executions.' : 'Starting needs the E2E worker.'} `
                  : ''}
                {sentence(failure.text)}
                {failure.field === 'e2e' ? (
                  <>
                    <span className="eval-ui-rv-foot">
                      {failure.unconfirmed
                        ? 'The run stays in progress on the card for a few minutes, so nothing can be started twice. Look for the executions in the E2E: if they exist, attach them when they finish; if not, start again once the card says it failed.'
                        : 'If one of the two executions was already started, it keeps running in the E2E and the card says so.'}
                    </span>
                    <span className="eval-ui-rv-inline-actions">
                      <Button type="button" variant="pill" size={size} onClick={onAttachExisting}>
                        <FlaskConical size={16} aria-hidden="true" />
                        Attach existing runs
                      </Button>
                      {failure.unconfirmed ? null : (
                        <Button type="submit" variant="primary" size={size}>
                          <RefreshCw size={16} aria-hidden="true" />
                          Try again
                        </Button>
                      )}
                    </span>
                  </>
                ) : null}
              </>
            }
          />
        ) : null}

        {current?.phase === 'refused' && current.field === 'other' ? (
          <>
            <FieldLine tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
              {sentence(current.text)}
            </FieldLine>
            <div>
              <Button type="button" variant="ghost" size={size} onClick={checkRefs}>
                Check again
              </Button>
            </div>
          </>
        ) : null}

        <FormField
          id={`${uid}-scenario`}
          label="Scenario"
          hint={plan.scenario_id ? 'from the plan' : 'the plan names none'}
          help={
            data.scenarios.phase === 'failed'
              ? "The E2E catalog couldn't be read; only the plan's scenario is offered."
              : plan.scenario_id
                ? undefined
                : 'The plan asks for a case the catalog may not have. Pick the closest one, or write the case in the E2E first.'
          }
        >
          <Selector
            id={`${uid}-scenario`}
            aria-label="Scenario"
            value={scenario || undefined}
            groups={groups}
            onChange={setScenario}
            loading={data.scenarios.phase === 'loading'}
            loadingMessage="Loading scenarios…"
            placeholder="Choose a scenario…"
            searchPlaceholder="Search scenarios…"
            emptyMessage="No scenario matches"
            disabled={starting}
          />
        </FormField>

        <FormField
          id={`${uid}-candidate`}
          label="Candidate"
          hint="Harness with the change"
          help="A branch, tag or commit that is pushed: the Docker stack builds the Harness from GitHub at this commit."
          line={
            <>
              <Message failure={at('candidate')} />
              <RefLine check={current} side="candidate" />
            </>
          }
        >
          <Input
            id={`${uid}-candidate`}
            className="eval-ui-val-mono"
            value={candidate}
            onBlur={checkRefs}
            onChange={(next) => {
              setCandidate(next)
              if (failure?.field === 'candidate') setFailure(undefined)
            }}
            placeholder="fix/registry-notice-scope"
            autoComplete="off"
            spellCheck={false}
            disabled={starting}
            aria-invalid={at('candidate') ? true : undefined}
          />
        </FormField>

        <FormField
          id={`${uid}-baseline`}
          label="Baseline"
          hint="Harness without the change"
          line={
            <>
              <Message failure={at('baseline')} />
              <RefLine check={current} side="baseline" />
            </>
          }
        >
          {baseline === null ? (
            <div className="eval-ui-rv-baseline">
              <span className="eval-ui-rv-baseline-default">Merge-base with origin/main</span>
              <Button type="button" variant="ghost" size={size} disabled={starting} onClick={() => setBaseline('')}>
                Change
              </Button>
            </div>
          ) : (
            <div className="eval-ui-rv-baseline">
              <Input
                id={`${uid}-baseline`}
                className="eval-ui-val-mono"
                value={baseline}
                onBlur={checkRefs}
                onChange={(next) => {
                  setBaseline(next)
                  if (failure?.field === 'baseline') setFailure(undefined)
                }}
                placeholder="a branch, tag or commit"
                autoComplete="off"
                spellCheck={false}
                disabled={starting}
                aria-invalid={at('baseline') ? true : undefined}
              />
              <Button type="button" variant="ghost" size={size} disabled={starting} onClick={() => setBaseline(null)}>
                Use the merge-base
              </Button>
            </div>
          )}
        </FormField>

        <div className="eval-ui-rv-pair">
          <FormField
            id={`${uid}-model`}
            label="Model and provider"
            help={
              observed
                ? chosen === observedKey
                  ? `The observed session ran with ${observed.model}.`
                  : `Not the model the observed session ran with (${observed.model} · ${observed.provider}).`
                : undefined
            }
            line={
              data.models.phase === 'failed' ? (
                <FieldLine tone="warn">The model catalog couldn't be read.</FieldLine>
              ) : provider && !proven(provider) ? (
                <FieldLine tone="warn">
                  The E2E hasn't run {provider} in Docker yet. The stack may not carry it, and the Docker build would be
                  spent before the executions fail.
                </FieldLine>
              ) : null
            }
          >
            <Select
              id={`${uid}-model`}
              value={chosen || undefined}
              onChange={setModel}
              options={options}
              placeholder={data.models.phase === 'loading' ? 'Loading models…' : 'Choose a model…'}
              disabled={starting || options.length === 0}
            />
          </FormField>
          <FormField
            id={`${uid}-runs`}
            label="Runs per side"
            help={runsError ? undefined : '1 to 20. The E2E advises at least 5.'}
            line={
              runsError ? (
                <FieldLine tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
                  {runsError}
                </FieldLine>
              ) : (
                <Message failure={at('runs')} />
              )
            }
          >
            <Input
              id={`${uid}-runs`}
              value={runs}
              onChange={setRuns}
              inputMode="numeric"
              disabled={starting}
              aria-invalid={runsError ? true : undefined}
            />
          </FormField>
        </div>

        {estimate ? (
          <StatusPanel
            variant="info"
            icon={<Clock className={uiClasses.icon} aria-hidden />}
            headline={[
              'Estimate',
              estimate.cost ? costApprox(estimate.cost.usd) : undefined,
              estimate.minutes ? minutesApprox(estimate.minutes.value) : undefined,
            ]
              .filter(Boolean)
              .join(' · ')}
            detail={
              <>
                {'runs' in runsParsed ? `${runsParsed.runs * 2} runs, ${runsParsed.runs} per side, ` : ''}of{' '}
                <span className="eval-ui-val-mono">{scenario}</span> with {modelId}. Both sides run in parallel.
                <span className="eval-ui-rv-foot">
                  The median of {estimate.executions} past Docker executions of this scenario with this model
                  {estimate.cost
                    ? `; cost from ${estimate.cost.from} of ${estimate.executions}`
                    : '; none reported a cost'}
                  {estimate.minutes && estimate.minutes.from < estimate.executions
                    ? `; time from ${estimate.minutes.from} of ${estimate.executions}`
                    : ''}
                  . The Docker build of a new commit adds minutes and isn't model cost.
                </span>
              </>
            }
          />
        ) : data.executions.phase === 'loading' ? null : (
          <StatusPanel
            variant="warn"
            icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
            headline="No estimate yet"
            detail={
              <>
                {data.executions.phase === 'failed' ? (
                  "The E2E's past executions could not be read."
                ) : (
                  <>
                    Fewer than {MIN_PAST} past Docker executions of{' '}
                    <span className="eval-ui-val-mono">{scenario || 'this scenario'}</span> with{' '}
                    {modelId || 'this model'} finished.
                  </>
                )}{' '}
                Starting still spends real model money; the cheapest start is {minimum} runs per side.
              </>
            }
          />
        )}

        <div className="eval-ui-val-block">
          <span className="eval-ui-val-label">
            {criterion && frozen ? <Lock size={16} aria-hidden="true" /> : null} {criterionLine}
          </span>
          {parsed.ok ? (
            <span className="eval-ui-val-quiet eval-ui-val-small">{criterionSummary(parsed.input)}</span>
          ) : null}
          {frozen ? (
            <span className="eval-ui-val-quiet eval-ui-val-small">It is locked: results already exist.</span>
          ) : editing ? (
            <div className="eval-ui-val-fields">
              <CriterionForm
                idPrefix={`${uid}-criterion`}
                draft={draft}
                onChange={setDraft}
                patterns={review.patterns}
                disabled={starting}
                showErrors
              />
            </div>
          ) : (
            <div>
              <Button type="button" variant="ghost" size={size} disabled={starting} onClick={() => setEditing(true)}>
                Edit
              </Button>
            </div>
          )}
          <Message failure={at('criterion')} />
        </div>

        {starting ? (
          <FieldLine
            role="status"
            icon={<LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />}
          >
            Starting the executions… Registering the criterion, then asking the E2E to start the baseline and the
            candidate.
          </FieldLine>
        ) : null}
      </div>
      <FormActions
        sheet={sheet}
        narrow={narrow}
        note={
          <span className="eval-ui-val-quiet eval-ui-val-small">
            {ready
              ? 'Nothing starts until you press Start.'
              : current?.phase === 'checking'
                ? 'Checking the commits…'
                : 'Fill in the fields marked above.'}
          </span>
        }
        cancel={
          <Button type="button" variant={sheet ? 'pill' : 'ghost'} size={size} disabled={starting} onClick={onCancel}>
            Cancel
          </Button>
        }
        submit={
          <Button
            type="submit"
            variant="primary"
            size={size}
            disabled={!ready || starting}
            aria-busy={starting || undefined}
          >
            {starting ? (
              <>
                <LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
                Starting…
              </>
            ) : (
              <>
                <FlaskConical size={16} aria-hidden="true" />
                Start 2 executions
              </>
            )}
          </Button>
        }
      />
    </form>
  )
}

export function StartValidationDialog(props: {
  api: EvalApi
  evaluationId: string
  review: SuggestionReview
  links: ValidationLink[]
  plan: ValidationPlan
  title: string
  observed: { model: string; provider: string } | undefined
  narrow: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  onStarted: (row: SuggestionReview) => void
  onRefresh: () => void
  onAttachExisting: () => void
}) {
  const { open, onOpenChange, title, narrow, ...form } = props
  const n = form.review.suggestion_index + 1
  const busy = useRef(false)
  const onBusy = useCallback((next: boolean) => {
    busy.current = next
  }, [])
  // The request is not interruptible: Escape, the overlay and Cancel wait for it.
  const requestClose = (next: boolean) => {
    if (!next && busy.current) return
    onOpenChange(next)
  }
  return (
    <DialogFrame
      open={open}
      onOpenChange={requestClose}
      title={`Validate S${n} in E2E`}
      description={title}
      narrow={narrow}
    >
      {({ sheet }) => (
        <StartForm
          {...form}
          sheet={sheet}
          narrow={narrow}
          onBusy={onBusy}
          onStarted={(row) => {
            busy.current = false
            onOpenChange(false)
            form.onStarted(row)
          }}
          onAttachExisting={() => {
            onOpenChange(false)
            form.onAttachExisting()
          }}
          onCancel={() => requestClose(false)}
        />
      )}
    </DialogFrame>
  )
}
