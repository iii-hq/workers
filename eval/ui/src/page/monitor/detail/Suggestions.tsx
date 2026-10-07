// What the analyst proposes: up to three suggestions, plus the proposals the
// monitor rejected. A suggestion is validated first by replaying the step where
// its behavior happened (the Validation section); the E2E plan, its criterion,
// runs and verdict stay behind "Non-regression in E2E". A suggestion carries
// what people decided about it: where it stands (the status badge). The
// monitor proposes and counts; it never moves a suggestion by itself.
import {
  Button,
  Card,
  Chip,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  IconButton,
  StatusPanel,
} from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import {
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  CircleAlert,
  Copy,
  Ellipsis,
  FileX,
  FlaskConical,
  MessageSquare,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useId, useState } from 'react'
import type { EvalApi } from '../../../api'
import { briefMarkdown, entryLabel, planMarkdown } from '../../../model'
import type {
  AnalysisAssets,
  AnalysisRecord,
  CodeRef,
  EntryRef,
  RejectedSuggestion,
  Snapshot,
  Suggestion,
  SuggestionReview,
  ValidationLink,
  ValidationPlan,
} from '../../../types'
import { AttachDialog } from './AttachDialog'
import { Disclosure, Inline, Pill, SectionHead } from './marks'
import {
  codeRefAria,
  codeRefCopy,
  codeRefLabel,
  latestValidation,
  locateEntry,
  MAX_CODE_REFS,
  MAX_SUGGESTIONS,
  plural,
} from './present'
import { RecurrencePanel } from './Recurrence'
import { ReplaySection } from './Replay'
import { evidenceLine, standing } from './reproduction-model'
import { CopyBriefButton, DraftButton, DraftNote } from './ReviewBrief'
import { CriterionBlock } from './ReviewCriterion'
import { LifecycleMenu } from './ReviewStatus'
import { VerdictDialog } from './ReviewVerdict'
import { criterionSentence, linksOf, OUTCOME_LABEL, OUTCOME_TONE, runActive, statusSentence } from './review-model'
import { StartValidationDialog } from './StartValidation'
import { ValidationProgress } from './StartValidationProgress'
import { ValidationPanel } from './ValidationPanel'

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="eval-ui-ad-field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

function Bullets({ items }: { items: string[] }) {
  return (
    <ul className="eval-ui-ad-list">
      {items.map((item, index) => (
        <li key={index}>
          <Inline text={item} />
        </li>
      ))}
    </ul>
  )
}

/** A cited entry: a button that jumps to it when it can be read, a plain tag otherwise. */
function EntryChip({
  snapshot,
  entry,
  narrow,
  onJump,
}: {
  snapshot: Snapshot | undefined
  entry: EntryRef
  narrow: boolean
  onJump: (entry: EntryRef) => void
}) {
  const label = entryLabel(snapshot, entry)
  const full = `${entry.session_id} · ${entry.entry_id}`
  if (!locateEntry(snapshot, entry)) {
    return (
      <Chip className="eval-ui-ad-ref" title={`${full} — not in the captured previews`}>
        {label}
      </Chip>
    )
  }
  return (
    <button type="button" className="eval-ui-ad-ref" title={full} onClick={() => onJump(entry)}>
      <span>{label}</span>
      {narrow ? <ChevronRight size={16} aria-hidden="true" /> : null}
    </button>
  )
}

/** Lines the analyst read, relative to the directory it ran in: a click copies `path:from-to`. */
function CodeRefChip({ reference }: { reference: CodeRef }) {
  const { state, copy } = useCopyFlash(codeRefCopy(reference))
  const Icon = state === 'copied' ? Check : state === 'failed' ? CircleAlert : Copy
  const label = codeRefLabel(reference)
  return (
    <>
      <button
        type="button"
        className="eval-ui-ad-ref"
        aria-label={codeRefAria(reference)}
        title={`Copy ${label} · lines as the investigation read them`}
        onClick={copy}
      >
        <span>{label}</span>
        <Icon size={16} aria-hidden="true" />
      </button>
      {/* Beside the button, not in it: its label would hide the announcement. */}
      <span className="eval-ui-sr-only" aria-live="polite">
        {state === 'copied' ? 'Copied' : state === 'failed' ? 'Copy failed' : ''}
      </span>
    </>
  )
}

function PlanHead({ plan, heading }: { plan: ValidationPlan; heading: boolean }) {
  return (
    <>
      {heading ? (
        <h4 className="eval-ui-ad-plan-title">Non-regression in E2E</h4>
      ) : (
        <span className="eval-ui-ad-plan-title">Non-regression in E2E</span>
      )}
      {plan.scenario_id ? (
        <Chip className="eval-ui-ad-mono-chip">{plan.scenario_id}</Chip>
      ) : (
        <span className="eval-ui-ad-mono-quiet">no scenario named</span>
      )}
    </>
  )
}

function PlanBody({ plan }: { plan: ValidationPlan }) {
  return (
    <dl className="eval-ui-ad-fields">
      <Field label="Reproduction">
        <Inline text={plan.reproduction} />
      </Field>
      <Field label="Invariants">
        <Bullets items={plan.invariants} />
      </Field>
      <Field label="Primary metric">
        <Inline text={plan.primary_metric} />
      </Field>
      <Field label="Expectation">
        <Inline text={plan.expectation} />
      </Field>
      {plan.non_regression_controls.length > 0 ? (
        <Field label={plan.non_regression_controls.length === 1 ? 'Control' : 'Controls'}>
          <Bullets items={plan.non_regression_controls} />
        </Field>
      ) : null}
    </dl>
  )
}

function CopyPlan({ title, plan, narrow }: { title: string; plan: ValidationPlan; narrow: boolean }) {
  const { state, copy } = useCopyFlash(planMarkdown(title, plan))
  const Icon = state === 'copied' ? Check : Copy
  return (
    <Button variant="ghost" size={narrow ? 'lg' : 'sm'} onClick={copy}>
      <Icon size={16} aria-hidden="true" />
      <span aria-live="polite">
        {state === 'copied' ? 'Copied' : state === 'failed' ? 'Copy failed' : 'Copy E2E plan'}
      </span>
    </Button>
  )
}

function SuggestionCard({
  api,
  evaluationId,
  record,
  index,
  suggestion,
  review: stored,
  snapshot,
  codeRoot,
  validations,
  narrow,
  terminal,
  onJump,
  onAttached,
  onReviewed,
  onDraft,
  onOpenE2e,
}: {
  api: EvalApi
  evaluationId: string
  record: AnalysisRecord
  index: number
  suggestion: Suggestion
  review: SuggestionReview
  snapshot: Snapshot | undefined
  codeRoot: string | undefined
  validations: ValidationLink[]
  narrow: boolean
  terminal: boolean
  onJump: (entry: EntryRef) => void
  onAttached: (link: ValidationLink) => void
  /** Something people decide changed on this suggestion: read the analysis again. */
  onReviewed: () => void
  /** Opens an unsent chat draft; `undefined` when this console cannot. */
  onDraft?: (draft: { text: string; title: string }) => void
  onOpenE2e?: () => void
}) {
  const titleId = useId()
  const [attachOpen, setAttachOpen] = useState(false)
  const [startOpen, setStartOpen] = useState(false)
  const [verdictOpen, setVerdictOpen] = useState(false)
  const [drafted, setDrafted] = useState(false)
  const [copied, setCopied] = useState('')
  const [error, setError] = useState<string | null>(null)
  // What a change answered shows at once; the analysis is read again behind it.
  const [review, setReview] = useState(stored)
  useEffect(() => setReview(stored), [stored])
  const saved = (row: SuggestionReview) => {
    setReview(row)
    setError(null)
    onReviewed()
  }

  const links = validations.filter((link) => link.suggestion_index === index)
  const latest = latestValidation(validations, index)
  const size = narrow ? 'lg' : 'sm'
  const run = review.run
  const running = runActive(run)
  const brief = briefMarkdown({
    analysisId: evaluationId,
    sessionId: record.session_id,
    turnId: record.turn_id,
    harnessVersion: record.harness_version,
    index,
    suggestion,
    status: statusSentence(review.lifecycle),
    criterion: criterionSentence(review.criterion),
    replay: evidenceLine(standing(suggestion.check, review.reproductions)),
    codeRoot,
  })
  const draft = () => {
    onDraft?.({ text: brief, title: `Implement: ${suggestion.title}` })
    setDrafted(true)
  }
  const copy = (what: string, text: string) => {
    void copyText(text).then((ok) => setCopied(ok ? `${what} copied` : 'Copy failed'))
  }
  // The validation panel the card shows: progress while it moves, or when it failed and nothing was attached since.
  const showProgress =
    run !== undefined &&
    (running || (run.state === 'failed' && !links.some((link) => link.attached_at >= run.started_at)))
  // The E2E section opens by itself once E2E work exists for the suggestion.
  const e2eActive = Boolean(run || links.length > 0 || review.criterion || review.verdict)

  const attach = terminal ? (
    <Button variant="ghost" size={size} onClick={() => setAttachOpen(true)}>
      <FlaskConical size={16} aria-hidden="true" />
      {links.length > 0 ? 'Replace E2E runs' : 'Attach E2E runs'}
    </Button>
  ) : null
  const validate = terminal ? (
    <Button
      variant="pill"
      size={size}
      className="eval-ui-rv-validate"
      disabled={running}
      title={running ? 'A validation of this suggestion is in progress' : undefined}
      onClick={() => setStartOpen(true)}
    >
      <FlaskConical size={16} aria-hidden="true" />
      Validate in E2E
    </Button>
  ) : null

  const overflow = (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <IconButton label="More actions" className="eval-ui-ad-more eval-ui-rv-more">
          <Ellipsis size={16} aria-hidden="true" />
        </IconButton>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => copy('Brief', brief)}>
          <Copy size={16} aria-hidden="true" />
          Copy implementation brief
        </DropdownMenuItem>
        {onDraft && codeRoot ? (
          <DropdownMenuItem onSelect={draft}>
            <MessageSquare size={16} aria-hidden="true" />
            Draft in chat
          </DropdownMenuItem>
        ) : null}
        {terminal ? <DropdownMenuSeparator /> : null}
        {terminal ? (
          <DropdownMenuItem disabled={running} onSelect={() => window.setTimeout(() => setStartOpen(true), 0)}>
            <FlaskConical size={16} aria-hidden="true" />
            Validate in E2E
          </DropdownMenuItem>
        ) : null}
        {terminal ? (
          <DropdownMenuItem onSelect={() => window.setTimeout(() => setAttachOpen(true), 0)}>
            <FlaskConical size={16} aria-hidden="true" />
            {links.length > 0 ? 'Replace E2E runs' : 'Attach E2E runs'}
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem onSelect={() => copy('Plan', planMarkdown(suggestion.title, suggestion.validation))}>
          <Copy size={16} aria-hidden="true" />
          Copy E2E plan
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )

  return (
    <Card className="eval-ui-ad-card" role="article" aria-labelledby={titleId}>
      <div className="eval-ui-ad-card-head">
        <span className="eval-ui-ad-mono-quiet">S{index + 1}</span>
        <LifecycleMenu
          target={{ api, evaluationId, index, title: suggestion.title }}
          lifecycle={review.lifecycle}
          narrow={narrow}
          onSaved={saved}
          onError={setError}
        />
        {review.verdict ? (
          <Pill tone={OUTCOME_TONE[review.verdict.outcome]} strong={OUTCOME_TONE[review.verdict.outcome] === 'neutral'}>
            {OUTCOME_LABEL[review.verdict.outcome]}
          </Pill>
        ) : null}
        {latest && !latest.comparability.comparable ? <Pill tone="alert">Not comparable</Pill> : null}
        {narrow ? (
          overflow
        ) : (
          <div className="eval-ui-ad-card-actions">
            <CopyBriefButton brief={brief} narrow={narrow} />
            {onDraft ? <DraftButton disabled={!codeRoot} narrow={narrow} onClick={draft} /> : null}
          </div>
        )}
      </div>
      <h3 id={titleId} className="eval-ui-ad-card-title">
        {suggestion.title}
      </h3>
      <span className="eval-ui-sr-only" aria-live="polite">
        {copied}
      </span>
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
      {onDraft ? <DraftNote drafted={drafted} codeRoot={codeRoot} /> : null}
      <dl className="eval-ui-ad-fields">
        <Field label="Observation">
          <Inline text={suggestion.observation} />
        </Field>
        <Field label="Hypothesis">
          <Inline text={suggestion.hypothesis} />
        </Field>
        <Field label="Harness area">
          <Inline text={suggestion.harness_component} />
        </Field>
        <Field label="Proposed change">
          <Inline text={suggestion.proposed_change} />
        </Field>
        <Field label="Expected effect">
          <Inline text={suggestion.expected_effect} />
        </Field>
        {suggestion.evidence.length > 0 ? (
          <Field label="Evidence">
            <div className="eval-ui-ad-refs">
              {suggestion.evidence.map((entry) => (
                <EntryChip
                  key={`${entry.session_id}/${entry.entry_id}`}
                  snapshot={snapshot}
                  entry={entry}
                  narrow={narrow}
                  onJump={onJump}
                />
              ))}
            </div>
          </Field>
        ) : null}
        {suggestion.code_refs.length > 0 ? (
          <Field label="Code">
            <div className="eval-ui-ad-refs">
              {suggestion.code_refs.slice(0, MAX_CODE_REFS).map((reference, index) => (
                <CodeRefChip key={index} reference={reference} />
              ))}
            </div>
          </Field>
        ) : null}
        <Field label="Limitations">
          <Inline text={suggestion.limitations} />
        </Field>
      </dl>
      {terminal ? (
        <ReplaySection
          api={api}
          evaluationId={evaluationId}
          index={index}
          suggestion={suggestion}
          review={review}
          snapshot={snapshot}
          narrow={narrow}
          onSaved={saved}
          onReviewed={onReviewed}
          onValidateInE2e={() => setStartOpen(true)}
        />
      ) : null}
      {/* The E2E path: for changes that act on every step, and for steps a replay cannot rebuild. */}
      <Disclosure
        className="eval-ui-ad-plan-disclosure"
        defaultOpen={e2eActive}
        summary={(open) => (
          <>
            <PlanHead plan={suggestion.validation} heading={false} />
            {open ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
          </>
        )}
      >
        <div className="eval-ui-ad-plan-body eval-ui-ad-plan">
          <div className="eval-ui-ad-card-actions">
            <CopyPlan title={suggestion.title} plan={suggestion.validation} narrow={narrow} />
            {attach}
            {validate}
          </div>
          <PlanBody plan={suggestion.validation} />
        </div>
        <CriterionBlock
          api={api}
          evaluationId={evaluationId}
          review={review}
          links={validations}
          planScenario={suggestion.validation.scenario_id}
          narrow={narrow}
          onSaved={saved}
          onError={setError}
        />
        {showProgress && run ? (
          <ValidationProgress
            api={api}
            evaluationId={evaluationId}
            index={index}
            run={run}
            readingPattern={review.criterion?.pattern?.split(':')[0]}
            narrow={narrow}
            onSettled={onReviewed}
            onStartAgain={() => setStartOpen(true)}
            onAttachOther={() => setAttachOpen(true)}
            onOpenE2e={onOpenE2e}
          />
        ) : null}
        <ValidationPanel
          links={linksOf(review, validations)}
          review={review}
          plan={suggestion.validation}
          narrow={narrow}
          onVerdict={() => setVerdictOpen(true)}
          onOpenE2e={onOpenE2e}
        />
      </Disclosure>
      {review.lifecycle.status === 'shipped' ? (
        <RecurrencePanel
          api={api}
          evaluationId={evaluationId}
          review={review}
          title={suggestion.title}
          narrow={narrow}
          onSaved={saved}
        />
      ) : null}
      {terminal ? (
        <>
          <AttachDialog
            api={api}
            evaluationId={evaluationId}
            suggestionIndex={index}
            suggestionTitle={suggestion.title}
            scenarioId={suggestion.validation.scenario_id}
            onOpenE2e={onOpenE2e}
            narrow={narrow}
            open={attachOpen}
            onOpenChange={setAttachOpen}
            onAttached={(link) => {
              setAttachOpen(false)
              onAttached(link)
            }}
          />
          <StartValidationDialog
            api={api}
            evaluationId={evaluationId}
            review={review}
            links={validations}
            plan={suggestion.validation}
            title={suggestion.title}
            observed={
              snapshot?.observed_model && snapshot.observed_provider
                ? { model: snapshot.observed_model, provider: snapshot.observed_provider }
                : undefined
            }
            narrow={narrow}
            open={startOpen}
            onOpenChange={setStartOpen}
            onStarted={saved}
            onRefresh={onReviewed}
            onAttachExisting={() => setAttachOpen(true)}
          />
        </>
      ) : null}
      {verdictOpen ? (
        <VerdictDialog
          api={api}
          evaluationId={evaluationId}
          review={review}
          links={validations}
          plan={suggestion.validation}
          title={suggestion.title}
          narrow={narrow}
          open={verdictOpen}
          onOpenChange={setVerdictOpen}
          onSaved={saved}
        />
      ) : null}
    </Card>
  )
}

function Rejected({ items }: { items: RejectedSuggestion[] }) {
  return (
    <Disclosure
      className="eval-ui-ad-rejected"
      summary={(open) => (
        <>
          <FileX size={16} aria-hidden="true" />
          <span className="eval-ui-ad-disclosure-title">
            {plural(items.length, 'suggestion')} rejected by validation
          </span>
          <span className="eval-ui-ad-disclosure-hint">
            {open ? 'Hide' : 'Show why'}
            {open ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
          </span>
        </>
      )}
    >
      <div className="eval-ui-ad-rejected-body">
        <p className="eval-ui-ad-quiet">
          These proposals were never shown as suggestions. Each broke a rule the monitor checks before it keeps one, so
          they can't be attached to E2E runs or validated.
        </p>
        {items.map((item) => (
          <div key={item.index} className="eval-ui-ad-rejected-item">
            <div className="eval-ui-ad-rejected-head">
              <span className="eval-ui-ad-mono-quiet">Proposal {item.index + 1}</span>
              <span className="eval-ui-ad-rejected-title">{item.title}</span>
            </div>
            <ul className="eval-ui-ad-reasons">
              {item.reasons.map((reason, index) => (
                <li key={index}>
                  <CircleAlert size={16} aria-hidden="true" />
                  <span>{reason}</span>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
    </Disclosure>
  )
}

export function Suggestions({
  api,
  evaluationId,
  record,
  assets,
  reviews,
  narrow,
  terminal,
  onJump,
  onAttached,
  onReviewed,
  onDraft,
  onOpenE2e,
}: {
  api: EvalApi
  evaluationId: string
  record: AnalysisRecord
  assets: AnalysisAssets
  /** One row per suggestion: what people decided about it. */
  reviews: SuggestionReview[]
  narrow: boolean
  terminal: boolean
  onJump: (entry: EntryRef) => void
  onAttached: (link: ValidationLink) => void
  onReviewed: () => void
  /** Opens an unsent chat draft; `undefined` when this console cannot. */
  onDraft?: (draft: { text: string; title: string }) => void
  /** Shows the E2E page; `undefined` when this console cannot. */
  onOpenE2e?: () => void
}) {
  const headingId = useId()
  const investigation = assets.investigation
  if (!investigation) return null
  const { suggestions, rejected } = investigation
  // Nothing proposed and nothing rejected: the notice above already says so.
  if (suggestions.length === 0 && rejected.length === 0) return null

  return (
    <section aria-labelledby={headingId} className="eval-ui-ad-section">
      <SectionHead id={headingId} title="Suggestions" meta={`${suggestions.length} of ${MAX_SUGGESTIONS} max`} />
      {suggestions.length === 0 ? (
        <div className="eval-ui-ad-panel eval-ui-ad-inline-notice">
          <CircleAlert size={16} aria-hidden="true" />
          <div>
            <p className="eval-ui-ad-strong">No suggestion passed validation</p>
            <p className="eval-ui-ad-quiet">
              The analyst proposed {plural(rejected.length, 'change')}. None met the monitor's checks, so nothing is
              shown as a suggestion.
            </p>
          </div>
        </div>
      ) : (
        suggestions.map((suggestion, index) => (
          <SuggestionCard
            key={index}
            api={api}
            evaluationId={evaluationId}
            record={record}
            index={index}
            suggestion={suggestion}
            review={reviews[index]}
            snapshot={assets.snapshot}
            codeRoot={investigation.code_root}
            validations={assets.validations}
            narrow={narrow}
            terminal={terminal}
            onJump={onJump}
            onAttached={onAttached}
            onReviewed={onReviewed}
            onDraft={onDraft}
            onOpenE2e={onOpenE2e}
          />
        ))
      )}
      {rejected.length > 0 ? <Rejected items={rejected} /> : null}
    </section>
  )
}
