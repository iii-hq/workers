// "Attach E2E runs to S1": link a baseline and a candidate E2E execution to a
// suggestion. The user picks both runs from the E2E service's list. The
// dialog looks the pair up (a dry run of `eval::attach-validation`), shows what
// the E2E service holds and whether the runs are comparable, and only then
// saves. It never grades the runs or starts a campaign.
import {
  BottomSheet,
  BottomSheetContent,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import {
  Check,
  CircleAlert,
  ExternalLink,
  Info,
  LoaderCircle,
  Minus,
  Plug,
  RefreshCw,
  TriangleAlert,
} from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useId, useRef, useState } from 'react'
import type { EvalApi } from '../../../api'
import type { E2eExecution, ValidationLink } from '../../../types'
import { type E2eRun, runGroups, withPicked } from './e2e-runs'
import { Inline } from './marks'
import { RunPicker } from './RunPicker'
import { usePhoneViewport } from './review-parts'
import { useRunList } from './use-run-list'
import {
  classifyAttachFailure,
  failedField,
  idState,
  type LookupView,
  lookupKey,
  NOT_FOUND_MESSAGE,
  primaryAction,
  SAME_ID_MESSAGE,
  type Side,
} from './validation-lookup'
import { CheckList, HarnessLine, Mismatches } from './validation-parts'
import { foundSummary, mismatches, noRunsNotice, planHint } from './validation-view'

function Line({
  tone,
  icon,
  role,
  children,
}: {
  tone?: 'ok' | 'alert' | 'warn'
  icon: ReactNode
  role?: 'status' | 'alert'
  children: ReactNode
}) {
  return (
    <span className="eval-ui-val-line" data-tone={tone} role={role}>
      {icon}
      <span>{children}</span>
    </span>
  )
}

/** The plan's line above the pickers, with the way out to the E2E page. */
function PlanHint({
  scenarioId,
  promoted,
  disabled,
  narrow,
  onOpenE2e,
}: {
  scenarioId: string | null
  /** Running the case in E2E is the next step: Open E2E is a button, not a quiet link. */
  promoted: boolean
  /** Leaving the dialog is refused while a save is in flight. */
  disabled: boolean
  narrow: boolean
  /** Hidden when the console cannot open the E2E page. */
  onOpenE2e: (() => void) | undefined
}) {
  return (
    <div className="eval-ui-val-hint">
      <p className="eval-ui-val-hint-text">
        <Inline text={planHint(scenarioId)} />
      </p>
      {onOpenE2e ? (
        <Button
          type="button"
          variant={promoted ? 'pill' : 'ghost'}
          size={narrow ? 'lg' : 'sm'}
          disabled={disabled}
          onClick={onOpenE2e}
        >
          Open E2E
          <ExternalLink className={uiClasses.icon} aria-hidden />
        </Button>
      ) : null}
    </div>
  )
}

/** The E2E service holds no runs at all. */
function NoRunsPanel({ scenarioId }: { scenarioId: string | null }) {
  const notice = noRunsNotice(scenarioId)
  return (
    <StatusPanel
      variant="info"
      role="status"
      icon={<Info className={uiClasses.icon} aria-hidden />}
      headline={notice.headline}
      detail={notice.detail}
    />
  )
}

/** The run list did not load; Try again is the dialog's primary action. */
function RunsDownPanel() {
  return (
    <StatusPanel
      variant="warn"
      role="alert"
      icon={<Plug className={uiClasses.icon} aria-hidden />}
      headline="E2E service unavailable"
      detail="The run list needs the E2E worker, and it isn't answering. There is nothing to choose from, and nothing was saved."
    />
  )
}

interface FormProps {
  api: EvalApi
  evaluationId: string
  suggestionIndex: number
  /** The plan's scenario; the hint, the run groups and the no-pair reasons are built from it. */
  scenarioId: string | null
  onOpenE2e: (() => void) | undefined
  narrow: boolean
  /** Phone bottom sheet: the actions stack, Attach first. */
  sheet: boolean
  onAttached: (link: ValidationLink) => void
  /** Close the dialog (after a save). */
  onDone: () => void
  /** Cancel and the overlay close are refused while a save is in flight. */
  onSaving: (saving: boolean) => void
  onCancel: () => void
}

/** `Found · Harness 1.8.39 · cc6b778`: the build from the run list when it has one, else what the lookup read. */
function foundLine(execution: E2eExecution | undefined, run: E2eRun | undefined) {
  const summary = run?.harness ? `Harness ${run.harness}` : execution ? foundSummary(execution) : ''
  return (
    <Line tone="ok" icon={<Check className={uiClasses.icon} aria-hidden />}>
      {summary ? (
        <>
          Found · <span className="eval-ui-val-mono">{summary}</span>
        </>
      ) : (
        'Found'
      )}
    </Line>
  )
}

function AttachForm({
  api,
  evaluationId,
  suggestionIndex,
  scenarioId,
  onOpenE2e,
  narrow,
  sheet,
  onAttached,
  onDone,
  onSaving,
  onCancel,
}: FormProps) {
  const uid = useId()
  const [baseline, setBaseline] = useState('')
  const [candidate, setCandidate] = useState('')
  const [lookup, setLookup] = useState<{ key: string; view: LookupView }>({ key: '', view: { phase: 'idle' } })
  const [saving, setSaving] = useState(false)
  const [attachError, setAttachError] = useState<string | null>(null)
  const latest = useRef(0)
  const requested = useRef('')
  const lastLink = useRef<ValidationLink | undefined>(undefined)
  const alive = useRef(true)
  // Set again on every mount: StrictMode mounts, unmounts and mounts a dev tree.
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])

  const { list, reload } = useRunList(api)
  const runs = list.phase === 'ready' ? list.runs : []
  const runById = (id: string) => runs.find((run) => run.id === id)

  const ids = idState(baseline, candidate)
  const key = lookupKey(baseline, candidate)
  // A result only counts for the ids it was asked about.
  const view: LookupView = lookup.key === key ? lookup.view : { phase: 'idle' }

  const start = useCallback(
    (force: boolean) => {
      if (idState(baseline, candidate) !== 'ready') return
      if (!force && requested.current === key) return
      requested.current = key
      const request = ++latest.current
      const current = (): boolean => alive.current && request === latest.current
      setLookup({ key, view: { phase: 'checking' } })
      api
        .attachValidation({
          evaluationId,
          suggestionIndex,
          baselineExecutionId: baseline.trim(),
          candidateExecutionId: candidate.trim(),
          dryRun: true,
        })
        .then(({ link }) => {
          if (!current()) return
          lastLink.current = link
          setLookup({ key, view: { phase: 'found', link } })
        })
        .catch(
          (error) =>
            current() &&
            setLookup({ key, view: { phase: 'failed', failure: classifyAttachFailure(errorMessage(error)) } }),
        )
    },
    [api, baseline, candidate, evaluationId, key, suggestionIndex],
  )

  // A pick is a choice, not typing: look the pair up as soon as both are there.
  useEffect(() => {
    start(false)
  }, [start])

  const attach = async () => {
    setSaving(true)
    onSaving(true)
    setAttachError(null)
    try {
      const { link } = await api.attachValidation({
        evaluationId,
        suggestionIndex,
        baselineExecutionId: baseline.trim(),
        candidateExecutionId: candidate.trim(),
        dryRun: false,
      })
      onSaving(false)
      onAttached(link)
      onDone()
    } catch (error) {
      onSaving(false)
      if (alive.current) {
        setAttachError(errorMessage(error))
        setSaving(false)
      }
    }
  }

  // A new pick invalidates the last save error and the lookup.
  const pick = (side: Side, next: string) => {
    if (next === (side === 'baseline' ? baseline : candidate)) return
    ;(side === 'baseline' ? setBaseline : setCandidate)(next)
    setAttachError(null)
  }

  const listFailed = list.phase === 'failed'
  const noRuns = list.phase === 'ready' && runs.length === 0
  const action = primaryAction(view, ids, listFailed)
  const submit = () => {
    if (saving || !action.enabled) return
    if (action.kind === 'retry') {
      if (listFailed) reload()
      else start(true)
    } else void attach()
  }

  const failure = view.phase === 'failed' ? view.failure : undefined
  const down = failure?.kind === 'unavailable'
  const ownIds = { baseline, candidate }
  const link = view.phase === 'found' ? view.link : undefined

  const fieldStatus = (side: Side): { status: ReactNode; invalid: boolean } => {
    if (side === 'candidate' && ids === 'same') {
      return {
        invalid: true,
        status: (
          <Line tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
            {SAME_ID_MESSAGE}
          </Line>
        ),
      }
    }
    if (failedField(failure, side, ownIds)) {
      return {
        invalid: true,
        status: (
          <Line tone="alert" role="alert" icon={<CircleAlert className={uiClasses.icon} aria-hidden />}>
            {NOT_FOUND_MESSAGE}
          </Line>
        ),
      }
    }
    if (view.phase === 'checking') {
      // One call looks up the pair; the run that did not change was found already.
      const kept = lastLink.current?.[side]
      if (kept && kept.execution_id === ownIds[side].trim()) {
        return { invalid: false, status: foundLine(kept, runById(kept.execution_id)) }
      }
      return {
        invalid: false,
        status: (
          <Line role="status" icon={<LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />}>
            Looking up in the E2E service…
          </Line>
        ),
      }
    }
    // The backend reads the baseline first: a candidate it could not find means the baseline was found.
    if (side === 'baseline' && failedField(failure, 'candidate', ownIds)) {
      return { invalid: false, status: foundLine(undefined, runById(ownIds[side].trim())) }
    }
    // One run picked, the other still empty: it came from the E2E service's own list, so it is found.
    const picked = ids === 'incomplete' ? runById(ownIds[side].trim()) : undefined
    if (picked) return { invalid: false, status: foundLine(undefined, picked) }
    if (link) return { invalid: false, status: foundLine(link[side], runById(link[side].execution_id)) }
    if (down) {
      return {
        invalid: false,
        status: <Line icon={<Minus className={uiClasses.icon} aria-hidden />}>Not checked</Line>,
      }
    }
    return { invalid: false, status: null }
  }

  const baselineField = fieldStatus('baseline')
  const candidateField = fieldStatus('candidate')
  const showChecks = !down && failure?.kind !== 'failed'
  // A missing id the fields cannot show (the backend named one we did not type).
  const unclaimed =
    failure?.kind === 'not_found' &&
    !failedField(failure, 'baseline', ownIds) &&
    !failedField(failure, 'candidate', ownIds)

  const picker = (side: Side, label: string, hint: string, status: { status: ReactNode; invalid: boolean }) => {
    const value = side === 'baseline' ? baseline : candidate
    const other = runById(side === 'baseline' ? candidate : baseline)
    return (
      <RunPicker
        id={`${uid}-${side}`}
        statusId={`${uid}-${side}-status`}
        label={label}
        hint={hint}
        value={value}
        onChange={(next) => pick(side, next)}
        groups={withPicked(runGroups(runs, scenarioId, other, side === 'baseline' ? 'candidate' : 'baseline'), value)}
        loading={list.phase === 'loading'}
        placeholder={
          list.phase === 'loading'
            ? 'Loading runs…'
            : listFailed
              ? 'Runs unavailable'
              : noRuns
                ? 'No runs to choose from'
                : 'Choose a run…'
        }
        disabled={saving || listFailed || noRuns}
        invalid={status.invalid}
        status={status.status}
      />
    )
  }

  const cancelButton = (
    <Button
      type="button"
      variant={sheet ? 'pill' : 'ghost'}
      size={narrow ? 'lg' : 'sm'}
      disabled={saving}
      onClick={onCancel}
    >
      Cancel
    </Button>
  )
  const submitButton = (
    <Button
      type="submit"
      variant={action.kind === 'attach_anyway' ? 'pill' : 'primary'}
      size={narrow ? 'lg' : 'sm'}
      disabled={saving || !action.enabled}
      aria-busy={saving || undefined}
    >
      {saving ? (
        <>
          <LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
          Attaching…
        </>
      ) : action.kind === 'retry' ? (
        <>
          <RefreshCw className={uiClasses.icon} aria-hidden />
          Try again
        </>
      ) : action.kind === 'attach_anyway' ? (
        'Attach anyway'
      ) : (
        'Attach'
      )}
    </Button>
  )

  return (
    <form
      className="eval-ui-val-form"
      data-narrow={narrow || undefined}
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        submit()
      }}
    >
      <div className="eval-ui-val-fields">
        <PlanHint
          scenarioId={scenarioId}
          promoted={noRuns || listFailed}
          disabled={saving}
          narrow={narrow}
          onOpenE2e={onOpenE2e}
        />

        {picker('baseline', 'Baseline execution', 'Harness without the change', baselineField)}
        {picker('candidate', 'Candidate execution', 'Harness with the change', candidateField)}

        {listFailed ? <RunsDownPanel /> : null}
        {noRuns ? <NoRunsPanel scenarioId={scenarioId} /> : null}
        {failure?.kind === 'unavailable' ? (
          <StatusPanel
            variant="warn"
            role="alert"
            icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
            headline="E2E service unavailable"
            detail="Linking runs needs the E2E worker, and it isn't answering. The IDs can't be checked, and nothing was saved."
          />
        ) : null}
        {failure?.kind === 'failed' || (failure && unclaimed) ? (
          <StatusPanel
            variant="alert"
            role="alert"
            icon={<CircleAlert className={uiClasses.icon} aria-hidden />}
            headline="Couldn't check these executions"
            detail={failure.message}
          />
        ) : null}

        {link ? <Comparability link={link} /> : null}
        {showChecks && !link ? (
          <div className="eval-ui-val-block">
            <span className="eval-ui-val-label">Must match in both runs</span>
            <span className="eval-ui-val-quiet eval-ui-val-small">
              {ids === 'same' ? 'Checked once the two executions differ.' : 'Checked once both executions are found.'}
            </span>
          </div>
        ) : null}

        {attachError ? (
          <StatusPanel
            variant="alert"
            role="alert"
            icon={<CircleAlert className={uiClasses.icon} aria-hidden />}
            headline="Couldn't attach the runs"
            detail={attachError}
          />
        ) : null}
      </div>

      <div className="eval-ui-val-actions" data-stacked={narrow || undefined} data-sheet={sheet || undefined}>
        {sheet ? (
          <>
            {submitButton}
            {cancelButton}
          </>
        ) : (
          <>
            {cancelButton}
            {submitButton}
          </>
        )}
      </div>
    </form>
  )
}

/** What the lookup says about the pair: all identity checks match, or these differ. */
function Comparability({ link }: { link: ValidationLink }) {
  const { comparable, checks } = link.comparability
  if (comparable) {
    return (
      <div className="eval-ui-val-block">
        <span className="eval-ui-val-label">Must match in both runs</span>
        <CheckList checks={checks} values={false} />
        <span className="eval-ui-val-quiet eval-ui-val-small">
          <HarnessLine baseline={link.baseline} candidate={link.candidate} short />
        </span>
      </div>
    )
  }
  const different = mismatches(checks)
  const same = checks.length - different.length
  return (
    <>
      <StatusPanel
        variant="alert"
        role="alert"
        icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
        headline="Not comparable"
        detail={
          <>
            Baseline and candidate must differ only in the Harness version. These values differ.
            <Mismatches checks={different} />
            {same > 0 ? (
              <span className="eval-ui-val-line eval-ui-val-same" data-tone="ok">
                <Check className={uiClasses.icon} aria-hidden />
                {same} other {same === 1 ? 'check matches' : 'checks match'}
              </span>
            ) : null}
          </>
        }
      />
      <p className="eval-ui-val-note">
        You can still attach them. The link is saved and shown as Not comparable, with these differences.
      </p>
    </>
  )
}

export interface AttachDialogProps {
  api: EvalApi
  evaluationId: string
  suggestionIndex: number
  suggestionTitle: string
  /** The plan's scenario (`validation.scenario_id`); `null` when the analyst asked for a new case. */
  scenarioId: string | null
  /** Shows the E2E page; `undefined` on a console that cannot, and the control hides. */
  onOpenE2e?: () => void
  /** The pane is narrow: touch-sized controls, a bottom sheet on a phone. */
  narrow: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  onAttached: (link: ValidationLink) => void
}

export function AttachDialog({
  api,
  evaluationId,
  suggestionIndex,
  suggestionTitle,
  scenarioId,
  onOpenE2e,
  narrow,
  open,
  onOpenChange,
  onAttached,
}: AttachDialogProps) {
  const phone = usePhoneViewport()
  const sheet = narrow && phone
  const saving = useRef(false)
  const title = `Attach E2E runs to S${suggestionIndex + 1}`

  // The save is not interruptible: a close request (Escape, overlay, Cancel) waits.
  const requestClose = (next: boolean) => {
    if (!next && saving.current) return
    onOpenChange(next)
  }

  const form = (
    <AttachForm
      api={api}
      evaluationId={evaluationId}
      suggestionIndex={suggestionIndex}
      scenarioId={scenarioId}
      // The dialog is modal: leave it to work on the E2E page, and come back to a fresh list.
      onOpenE2e={
        onOpenE2e &&
        (() => {
          onOpenE2e()
          onOpenChange(false)
        })
      }
      narrow={narrow}
      sheet={sheet}
      onAttached={onAttached}
      onDone={() => onOpenChange(false)}
      onSaving={(next) => {
        saving.current = next
      }}
      onCancel={() => requestClose(false)}
    />
  )

  if (sheet) {
    return (
      <BottomSheet open={open} onOpenChange={requestClose}>
        <BottomSheetContent className="eval-ui-val-sheet" heading={title} description={suggestionTitle}>
          {form}
        </BottomSheetContent>
      </BottomSheet>
    )
  }
  return (
    <Dialog open={open} onOpenChange={requestClose}>
      <DialogContent className="eval-ui-val-dialog">
        <div className="eval-ui-val-header">
          <DialogTitle className="eval-ui-val-title">{title}</DialogTitle>
          <DialogDescription className="eval-ui-val-desc">{suggestionTitle}</DialogDescription>
        </div>
        {form}
      </DialogContent>
    </Dialog>
  )
}
