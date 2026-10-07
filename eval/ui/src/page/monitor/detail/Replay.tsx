// The Validation section of a suggestion (VALIDATION.md §4): replay the step
// where the behavior happened, test the proposed change at that same step,
// then approve or dismiss. One action at a time, no form by default: the
// decision point, the signal and the change come from the suggestion. Only
// "Test change" takes free input, and it is pre-filled.
import { Button, Checkbox, Select, SegmentedControl, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ChevronDown, ChevronUp, LoaderCircle, TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useId, useState } from 'react'
import type { EvalApi } from '../../../api'
import { entryLabel } from '../../../model'
import type {
  ChangeEdit,
  EntryRef,
  Fidelity,
  Reply,
  ReproducePreview,
  Reproduction,
  Snapshot,
  Suggestion,
  SuggestionCheck,
  SuggestionReview,
} from '../../../types'
import { Pill } from './marks'
import { formatCostShort } from './present'
import {
  actionCounts,
  compare,
  DEFAULT_SAMPLES,
  estimateCost,
  evidenceLine,
  FINAL_ANSWER,
  MAX_SAMPLES,
  MORE_SAMPLES,
  percent,
  reproductionSentence,
  signalLabel,
  type Standing,
  standing as standingOf,
  stepLabel,
  tally,
} from './reproduction-model'
import { DialogFrame, FieldLine, FormActions, FormField, TextArea } from './review-parts'

const SYSTEM_PROMPT = 'system_prompt'
/** Replies the result shows before "Show all". */
const SHOWN_REPLIES = 3

// ---------------------------------------------------------------------------
// Small pieces
// ---------------------------------------------------------------------------

function FidelityLine({ fidelity }: { fidelity: Fidelity }) {
  if (fidelity.level === 'exact') {
    return <Pill tone="ok">Reconstruction matches the original</Pill>
  }
  const off = fidelity.off_ratio === undefined ? '' : ` · ${(fidelity.off_ratio * 100).toFixed(1)}% off`
  return (
    <div className="eval-ui-rp-fidelity">
      <Pill tone="warn">Approximate reconstruction{off}</Pill>
      {fidelity.reasons.length > 0 ? (
        <ul className="eval-ui-rp-reasons">
          {fidelity.reasons.map((reason, index) => (
            <li key={index}>{reason}</li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}

function CallLine({ reply }: { reply: Reply }) {
  if (reply.error) return <p className="eval-ui-rp-error">{reply.error}</p>
  return (
    <>
      {reply.thinking ? <p className="eval-ui-rp-thinking">{reply.thinking}</p> : null}
      {reply.calls.length === 0 ? (
        <p className="eval-ui-rp-text">{reply.text || 'Final answer, no call.'}</p>
      ) : (
        <ul className="eval-ui-rp-calls">
          {reply.calls.map((call, index) => (
            <li key={index}>
              <code className="eval-ui-rp-target">{call.target}</code>
              <code className="eval-ui-rp-payload">{call.payload}</code>
            </li>
          ))}
        </ul>
      )}
    </>
  )
}

function ReplyRow({ reply, label }: { reply: Reply; label: string }) {
  const tone = reply.error ? 'alert' : reply.signal === true ? 'accent' : 'neutral'
  const mark = reply.error ? 'Failed' : reply.signal === true ? 'Signal' : reply.signal === false ? 'No signal' : 'Unclear'
  return (
    <div className="eval-ui-rp-reply" data-signal={reply.signal === true || undefined}>
      <div className="eval-ui-rp-reply-head">
        <span className="eval-ui-ad-mono-quiet">{label}</span>
        <Pill tone={tone}>{mark}</Pill>
      </div>
      <CallLine reply={reply} />
    </div>
  )
}

/** The original step first, then the replies: those with the signal first. */
function Replies({ reproduction }: { reproduction: Reproduction }) {
  const [all, setAll] = useState(false)
  const ordered = [...reproduction.samples].sort(
    (one, two) => Number(two.signal === true) - Number(one.signal === true) || one.index - two.index,
  )
  const shown = all ? ordered : ordered.slice(0, SHOWN_REPLIES)
  return (
    <div className="eval-ui-rp-replies">
      {reproduction.original ? <ReplyRow reply={reproduction.original} label="Original step" /> : null}
      {shown.map((reply) => (
        <ReplyRow key={reply.index} reply={reply} label={`Sample ${reply.index + 1}`} />
      ))}
      {ordered.length > SHOWN_REPLIES ? (
        <Button variant="ghost" size="sm" onClick={() => setAll(!all)}>
          {all ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
          {all ? 'Show fewer' : `Show all ${ordered.length} samples`}
        </Button>
      ) : null}
    </div>
  )
}

/** `read the files directly (17) · listed a folder (7)`: what the replies did besides the signal. */
function ActionSummary({ reproduction }: { reproduction: Reproduction }) {
  const counts = [...actionCounts(reproduction).entries()].sort((one, two) => two[1] - one[1])
  if (counts.length === 0) return null
  return (
    <p className="eval-ui-rp-quiet">
      Calls across the samples:{' '}
      {counts.map(([action, count], index) => (
        <span key={action}>
          {index > 0 ? ' · ' : ''}
          {action === FINAL_ANSWER ? 'final answer' : <code className="eval-ui-ad-code">{action}</code>} ({count})
        </span>
      ))}
    </p>
  )
}

function Progress({ reproduction }: { reproduction: Reproduction }) {
  const done = reproduction.samples.length
  const text =
    reproduction.phase === 'classifying'
      ? 'Reading the signal in each reply'
      : `${reproduction.change_kind === 'none' ? 'Replaying' : 'Testing the change'} · ${done} of ${reproduction.requested}`
  return (
    <div className="eval-ui-rp-progress" role="status" aria-live="polite">
      <LoaderCircle size={16} className={uiClasses.spin} aria-hidden="true" />
      <span>{text}</span>
      <progress max={reproduction.requested} value={done} aria-hidden="true" />
    </div>
  )
}

// ---------------------------------------------------------------------------
// Older suggestions: name the step and the signal
// ---------------------------------------------------------------------------

type SignalKind = 'question' | 'contract_rediscovery' | 'repeated_error_call'

function CheckSetup({
  suggestion,
  snapshot,
  narrow,
  onReady,
}: {
  suggestion: Suggestion
  snapshot: Snapshot | undefined
  narrow: boolean
  onReady: (check: SuggestionCheck) => void
}) {
  const uid = useId()
  const steps = suggestion.evidence.filter((entry) => entry.entry_id.endsWith('_assistant'))
  const [point, setPoint] = useState(steps[0]?.entry_id)
  const [kind, setKind] = useState<SignalKind>('question')
  const [question, setQuestion] = useState('')
  if (steps.length === 0) {
    return (
      <p className="eval-ui-rp-quiet">
        None of this suggestion's evidence is a reply of the model, so there is no step to replay. Reanalyze the
        session, or validate it in E2E.
      </p>
    )
  }
  const ready = point !== undefined && (kind !== 'question' || question.trim() !== '')
  return (
    <div className="eval-ui-rp-setup">
      <p className="eval-ui-rp-quiet">
        This suggestion predates replays. Name the step where the behavior happened and how to recognize it.
      </p>
      <FormField id={`${uid}-point`} label="Decision point">
        <Select
          id={`${uid}-point`}
          value={point}
          onChange={setPoint}
          options={steps.map((entry) => {
            // `step 4`, with the turn only when two cited replies share a step number.
            const label = stepLabel(entry.entry_id)
            const shared = steps.filter((other) => stepLabel(other.entry_id) === label).length > 1
            return { value: entry.entry_id, label: shared ? `${label} · ${entry.entry_id.slice(2, 14)}` : label }
          })}
        />
      </FormField>
      <FormField label="Signal">
        <SegmentedControl
          aria-label="How to recognize the behavior"
          value={kind}
          onChange={setKind}
          variant="radio"
          options={[
            { value: 'question', label: 'A question', icon: false },
            { value: 'contract_rediscovery', label: 'Re-reads a contract', icon: false },
            { value: 'repeated_error_call', label: 'Repeats a failed call', icon: false },
          ]}
        />
      </FormField>
      {kind === 'question' ? (
        <FormField id={`${uid}-question`} label="Question about one reply" hint="yes or no">
          <TextArea
            id={`${uid}-question`}
            value={question}
            onChange={setQuestion}
            rows={2}
            maxLength={500}
            placeholder="Does the reply call the same function again with the same arguments?"
          />
        </FormField>
      ) : null}
      <div className="eval-ui-rp-actions" data-stacked={narrow || undefined}>
        <Button
          variant="primary"
          size={narrow ? 'lg' : 'sm'}
          disabled={!ready}
          onClick={() =>
            point &&
            onReady({
              decision_point: point,
              signal: kind === 'question' ? { question: question.trim() } : { rule: kind },
              change: [],
            })
          }
        >
          Use this check
        </Button>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Test change
// ---------------------------------------------------------------------------

function TestChangeDialog({
  open,
  onOpenChange,
  check,
  suggestion,
  snapshot,
  estimate,
  narrow,
  onRun,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  check: SuggestionCheck
  suggestion: Suggestion
  snapshot: Snapshot | undefined
  estimate: number | undefined
  narrow: boolean
  /** `proposed` when the person kept the suggestion's own edits. Resolves to an error message, or null. */
  onRun: (change: 'proposed' | ChangeEdit[]) => Promise<string | null>
}) {
  const uid = useId()
  const proposed = check.change[0]
  const [target, setTarget] = useState(proposed?.target ?? suggestion.evidence[0]?.entry_id ?? SYSTEM_PROMPT)
  const [remove, setRemove] = useState(proposed?.remove ?? false)
  const [find, setFind] = useState(proposed?.find ?? '')
  const [replace, setReplace] = useState(proposed?.replace ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const session = snapshot?.source_session_id ?? ''
  const targets = [
    ...new Set([...check.change.map((edit) => edit.target), ...suggestion.evidence.map((entry) => entry.entry_id)]),
  ].filter((id) => id !== check.decision_point)
  const label = (id: string) =>
    id === SYSTEM_PROMPT ? 'System prompt' : entryLabel(snapshot, { session_id: session, entry_id: id } as EntryRef)
  const edit: ChangeEdit = remove ? { target, remove: true } : { target, find, replace }
  const unchanged =
    proposed !== undefined &&
    proposed.target === target &&
    (proposed.remove ?? false) === remove &&
    (proposed.find ?? '') === find &&
    (proposed.replace ?? '') === replace
  const valid = remove ? target !== SYSTEM_PROMPT : find !== ''
  const run = async () => {
    setBusy(true)
    setError(null)
    const message = await onRun(unchanged ? 'proposed' : [edit, ...check.change.slice(1)])
    setBusy(false)
    if (message) setError(message)
    else onOpenChange(false)
  }
  return (
    <DialogFrame
      open={open}
      onOpenChange={onOpenChange}
      title="Test the change"
      description={`The same step, with this edit of what the model saw before ${stepLabel(check.decision_point)}.`}
      narrow={narrow}
    >
      {({ sheet }) => (
        <div className="eval-ui-rp-dialog">
          <div className="eval-ui-rp-proposed">
            <span className="eval-ui-val-label">Proposed change</span>
            <p>{suggestion.proposed_change}</p>
          </div>
          <FormField id={`${uid}-target`} label="Where">
            <Select
              id={`${uid}-target`}
              value={target}
              onChange={setTarget}
              options={[...targets, SYSTEM_PROMPT].map((id) => ({ value: id, label: label(id) }))}
            />
          </FormField>
          {target !== SYSTEM_PROMPT ? (
            <Checkbox
              className="eval-ui-rp-check"
              checked={remove}
              onChange={(event) => setRemove(event.target.checked)}
              label="Remove this entry (a notice, for example)"
            />
          ) : null}
          {remove ? null : (
            <>
              <FormField id={`${uid}-find`} label="Text the model saw" hint="exact">
                <TextArea id={`${uid}-find`} value={find} onChange={setFind} rows={3} />
              </FormField>
              <FormField
                id={`${uid}-replace`}
                label="Text it sees instead"
                line={
                  error ? (
                    <FieldLine tone="alert" role="alert" icon={<TriangleAlert size={16} aria-hidden="true" />}>
                      {error}
                    </FieldLine>
                  ) : null
                }
              >
                <TextArea id={`${uid}-replace`} value={replace} onChange={setReplace} rows={3} />
              </FormField>
            </>
          )}
          {remove && error ? (
            <FieldLine tone="alert" role="alert" icon={<TriangleAlert size={16} aria-hidden="true" />}>
              {error}
            </FieldLine>
          ) : null}
          <FormActions
            sheet={sheet}
            narrow={narrow}
            note={`${DEFAULT_SAMPLES} samples, compared with the base${estimate === undefined ? '' : ` · up to ${formatCostShort(estimate)}`}`}
            cancel={
              <Button variant="ghost" size={narrow ? 'lg' : 'sm'} onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
            }
            submit={
              <Button variant="primary" size={narrow ? 'lg' : 'sm'} disabled={!valid || busy} onClick={() => void run()}>
                {busy ? <LoaderCircle size={16} className={uiClasses.spin} aria-hidden="true" /> : null}
                Run
              </Button>
            }
          />
        </div>
      )}
    </DialogFrame>
  )
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

function ComparisonTable({ base, change }: { base: Reproduction; change: Reproduction }) {
  const result = compare(base, change)
  const calls = (value: number | null) => (value === null ? '—' : value.toFixed(1))
  return (
    <>
      <p className="eval-ui-rp-sentence">{result.sentence}</p>
      <table className="eval-ui-rp-table">
        <thead>
          <tr>
            <th scope="col" />
            <th scope="col">Base</th>
            <th scope="col">Change</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <th scope="row">Signal</th>
            <td>
              {result.base.positive}/{result.base.measured} ({percent(result.base.positive, result.base.measured)})
            </td>
            <td>
              {result.change.positive}/{result.change.measured} ({percent(result.change.positive, result.change.measured)})
            </td>
          </tr>
          <tr>
            <th scope="row">Calls per sample</th>
            <td>{calls(result.calls.base)}</td>
            <td>{calls(result.calls.change)}</td>
          </tr>
          <tr>
            <th scope="row">Cost</th>
            <td>{formatCostShort(base.cost_usd)}</td>
            <td>{formatCostShort(change.cost_usd)}</td>
          </tr>
        </tbody>
      </table>
      {result.sideEffects.map((effect) => (
        <p key={effect.action} className="eval-ui-rp-side">
          <TriangleAlert size={16} aria-hidden="true" />
          <span>
            Side effect:{' '}
            {effect.action === FINAL_ANSWER ? 'a final answer' : <code className="eval-ui-ad-code">{effect.action}</code>}{' '}
            in {effect.change} of {effect.changeN} samples (base: {effect.base} of {effect.baseN}).
          </span>
        </p>
      ))}
    </>
  )
}

// ---------------------------------------------------------------------------
// The section
// ---------------------------------------------------------------------------

export function ReplaySection({
  api,
  evaluationId,
  index,
  suggestion,
  review,
  snapshot,
  narrow,
  onSaved,
  onReviewed,
  onValidateInE2e,
}: {
  api: EvalApi
  evaluationId: string
  index: number
  suggestion: Suggestion
  review: SuggestionReview
  snapshot: Snapshot | undefined
  narrow: boolean
  onSaved: (row: SuggestionReview) => void
  /** A replay started or ended: read the analysis again. */
  onReviewed: () => void
  /** Opens the E2E validation, the path for what cannot be replayed. */
  onValidateInE2e: () => void
}) {
  const headingId = useId()
  const reproductions = review.reproductions ?? []
  const [chosen, setChosen] = useState<SuggestionCheck | undefined>(undefined)
  const check = suggestion.check ?? reproductions.at(-1)?.check ?? chosen
  const current: Standing = standingOf(check, reproductions)
  const [preview, setPreview] = useState<ReproducePreview | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [testOpen, setTestOpen] = useState(false)
  // "Run 30 more" extends the base, then the change: one replay at a time.
  const [extendChangeNext, setExtendChangeNext] = useState<string | null>(null)
  const size = narrow ? 'lg' : 'sm'
  const decided = review.lifecycle.status === 'accepted' || review.lifecycle.status === 'rejected'

  // Before the first replay: rebuild the request (free) to show its fidelity and price.
  const checkKey = check ? JSON.stringify(check) : ''
  useEffect(() => {
    if (!check || reproductions.length > 0) return
    let live = true
    setPreview(null)
    setPreviewError(null)
    api
      .reproduce(evaluationId, index, { check: suggestion.check ? undefined : check, dryRun: true })
      .then((response) => live && setPreview(response.preview ?? null))
      .catch((reason: unknown) => live && setPreviewError(errorMessage(reason)))
    return () => {
      live = false
    }
    // The check's content, not its identity, decides a new preview.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, evaluationId, index, checkKey, reproductions.length > 0])

  useEffect(() => {
    if (!extendChangeNext || current.stage === 'reproducing' || current.stage === 'testing') return
    const id = extendChangeNext
    setExtendChangeNext(null)
    void start({ extend: id, samples: MORE_SAMPLES })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [extendChangeNext, current.stage])

  const stepCost = preview?.step_cost_usd ?? reproductions[0]?.original?.usage.cost_usd
  const estimate = (samples: number) => estimateCost(stepCost, samples)
  const priced = (samples: number) => {
    const cost = estimate(samples)
    return cost === undefined ? '' : ` · up to ${formatCostShort(cost)}`
  }

  async function start(params: Parameters<EvalApi['reproduce']>[2]): Promise<string | null> {
    setBusy(true)
    setError(null)
    try {
      const response = await api.reproduce(evaluationId, index, {
        ...params,
        check: suggestion.check || params.extend ? undefined : check,
      })
      if (response.review) onSaved(response.review)
      onReviewed()
      return null
    } catch (reason) {
      const message = errorMessage(reason)
      setError(message)
      return message
    } finally {
      setBusy(false)
    }
  }

  async function decide(status: 'accepted' | 'rejected') {
    const line = evidenceLine(current) ?? 'Decided after the replay'
    setBusy(true)
    setError(null)
    try {
      const row = await api.review(
        evaluationId,
        index,
        status === 'accepted'
          ? { action: 'set_lifecycle', status, note: line }
          : { action: 'set_lifecycle', status, reason: line },
      )
      onSaved(row)
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setBusy(false)
    }
  }

  const more = (reproduction: Reproduction) => Math.min(MORE_SAMPLES, MAX_SAMPLES - reproduction.requested)
  const decisionButtons = (approvePrimary: boolean) =>
    decided ? null : (
      <>
        <Button variant="ghost" size={size} disabled={busy} onClick={() => void decide('rejected')}>
          Dismiss
        </Button>
        <Button
          variant={approvePrimary ? 'primary' : 'ghost'}
          size={size}
          disabled={busy}
          onClick={() => void decide('accepted')}
        >
          Approve
        </Button>
      </>
    )

  let body: ReactNode
  switch (current.stage) {
    case 'setup':
      body = <CheckSetup suggestion={suggestion} snapshot={snapshot} narrow={narrow} onReady={setChosen} />
      break
    case 'ready':
      body = previewError ? (
        <StatusPanel
          variant="warn"
          headline="This step cannot be replayed"
          detail={previewError}
          action={
            <Button variant="ghost" size={size} onClick={onValidateInE2e}>
              Validate in E2E
            </Button>
          }
        />
      ) : (
        <>
          {preview ? <FidelityLine fidelity={preview.fidelity} /> : null}
          {preview ? <ReplyRow reply={preview.original} label="Original step" /> : null}
          <div className="eval-ui-rp-actions" data-stacked={narrow || undefined}>
            <span className="eval-ui-rp-quiet">
              Replays {stepLabel(check?.decision_point ?? '')} with the same model and context. Nothing is executed.
            </span>
            <Button
              variant="primary"
              size={size}
              disabled={busy || !preview}
              onClick={() => void start({ change: { kind: 'none' }, samples: DEFAULT_SAMPLES })}
            >
              {busy || !preview ? <LoaderCircle size={16} className={uiClasses.spin} aria-hidden="true" /> : null}
              Reproduce · {DEFAULT_SAMPLES} samples{priced(DEFAULT_SAMPLES)}
            </Button>
          </div>
        </>
      )
      break
    case 'reproducing':
    case 'testing':
      body = (
        <>
          {current.current ? <Progress reproduction={current.current} /> : null}
          {current.stage === 'testing' && current.base ? (
            <p className="eval-ui-rp-sentence">{reproductionSentence(current.base)}</p>
          ) : null}
        </>
      )
      break
    case 'reproduced':
    case 'not_reproduced':
      body = current.base ? (
        <>
          <p className="eval-ui-rp-sentence">
            {reproductionSentence(current.base)}
            {current.stage === 'not_reproduced' ? ' This suggestion is probably not worth it.' : ''}{' '}
            <span className="eval-ui-rp-cost">{formatCostShort(current.base.cost_usd)}</span>
          </p>
          {current.base.fidelity ? <FidelityLine fidelity={current.base.fidelity} /> : null}
          <ActionSummary reproduction={current.base} />
          <Replies reproduction={current.base} />
          <div className="eval-ui-rp-actions" data-stacked={narrow || undefined}>
            {current.stage === 'reproduced' ? (
              <>
                <Button
                  variant="ghost"
                  size={size}
                  disabled={busy || more(current.base) === 0}
                  onClick={() => void start({ extend: current.base!.id, samples: more(current.base!) })}
                >
                  Run {more(current.base)} more
                </Button>
                {decisionButtons(false)}
                <Button
                  variant="primary"
                  size={size}
                  disabled={busy}
                  onClick={() => setTestOpen(true)}
                >
                  Test change
                </Button>
              </>
            ) : (
              <>
                <Button
                  variant="ghost"
                  size={size}
                  disabled={busy || more(current.base) === 0}
                  onClick={() => void start({ extend: current.base!.id, samples: more(current.base!) })}
                >
                  Run {more(current.base)} more
                </Button>
                {decided ? null : (
                  <Button variant="primary" size={size} disabled={busy} onClick={() => void decide('rejected')}>
                    Dismiss
                  </Button>
                )}
              </>
            )}
          </div>
        </>
      ) : null
      break
    case 'compared': {
      const { base, change } = current
      if (!base || !change) break
      const result = compare(base, change)
      const extra = Math.min(result.moreNeeded ?? MORE_SAMPLES, more(base), more(change))
      body = (
        <>
          <ComparisonTable base={base} change={change} />
          <Replies reproduction={change} />
          <div className="eval-ui-rp-actions" data-stacked={narrow || undefined}>
            <Button variant="ghost" size={size} disabled={busy} onClick={() => setTestOpen(true)}>
              Test another change
            </Button>
            {result.inconclusive && extra > 0 ? (
              <Button
                variant="primary"
                size={size}
                disabled={busy}
                onClick={() => {
                  setExtendChangeNext(change.id)
                  void start({ extend: base.id, samples: extra })
                }}
              >
                Run {extra} more per side
              </Button>
            ) : null}
            {decisionButtons(!(result.inconclusive && extra > 0))}
          </div>
        </>
      )
      break
    }
    case 'failed':
      body = current.current ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="The replay stopped"
          detail={`${current.current.error ?? 'It failed.'} ${tally(current.current).ok} of ${current.current.requested} replies are kept.`}
          action={
            <Button
              variant="ghost"
              size={size}
              disabled={busy}
              onClick={() => void start({ extend: current.current!.id, samples: 0 })}
            >
              Retry
            </Button>
          }
        />
      ) : null
      break
  }

  return (
    <section className="eval-ui-rp" aria-labelledby={headingId}>
      <div className="eval-ui-rp-head">
        <h4 id={headingId} className="eval-ui-ad-plan-title">
          Validation
        </h4>
        {check ? (
          <span className="eval-ui-rp-quiet">
            {stepLabel(check.decision_point)} · {signalLabel(check)}
          </span>
        ) : null}
      </div>
      {error ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline="That did not go through"
          detail={error}
          action={
            <Button size={size} variant="ghost" onClick={() => setError(null)}>
              Dismiss
            </Button>
          }
        />
      ) : null}
      {body}
      {decided ? (
        <p className="eval-ui-rp-quiet">
          {review.lifecycle.status === 'accepted' ? 'Approved' : 'Dismissed'}
          {review.lifecycle.history.at(-1)?.by ? ` by ${review.lifecycle.history.at(-1)?.by}` : ''}.
        </p>
      ) : null}
      {check && (current.stage === 'reproduced' || current.stage === 'compared') ? (
        <TestChangeDialog
          key={testOpen ? 'open' : 'closed'}
          open={testOpen}
          onOpenChange={setTestOpen}
          check={check}
          suggestion={suggestion}
          snapshot={snapshot}
          estimate={estimate(DEFAULT_SAMPLES)}
          narrow={narrow}
          onRun={(edits) =>
            start({
              change: edits === 'proposed' ? { kind: 'proposed' } : { kind: 'custom', edits },
              samples: DEFAULT_SAMPLES,
            })
          }
        />
      ) : null}
    </section>
  )
}
