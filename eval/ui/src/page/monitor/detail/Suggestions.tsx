// What the analyst proposes: up to three suggestions, each with its E2E plan,
// plus the proposals the monitor rejected. A suggestion stays "Not validated"
// until an external E2E verdict says otherwise, and none exists today.
import { Button, Card, CardHighlight, Chip } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, ChevronDown, ChevronRight, ChevronUp, CircleAlert, Copy, FileX, FlaskConical } from 'lucide-react'
import type { ReactNode } from 'react'
import { useId, useState } from 'react'
import type { EvalApi } from '../../../api'
import { entryLabel, planMarkdown } from '../../../model'
import type {
  AnalysisAssets,
  CodeRef,
  EntryRef,
  RejectedSuggestion,
  Snapshot,
  Suggestion,
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
        <h4 className="eval-ui-ad-plan-title">E2E plan</h4>
      ) : (
        <span className="eval-ui-ad-plan-title">E2E plan</span>
      )}
      {plan.scenario_id ? (
        <Chip className="eval-ui-ad-mono-chip">{plan.scenario_id}</Chip>
      ) : (
        <Pill tone="warn">New scenario needed</Pill>
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
    <Button variant={narrow ? 'pill' : 'ghost'} size={narrow ? 'lg' : 'sm'} onClick={copy}>
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
  index,
  suggestion,
  snapshot,
  validations,
  narrow,
  terminal,
  onJump,
  onAttached,
  onOpenE2e,
}: {
  api: EvalApi
  evaluationId: string
  index: number
  suggestion: Suggestion
  snapshot: Snapshot | undefined
  validations: ValidationLink[]
  narrow: boolean
  terminal: boolean
  onJump: (entry: EntryRef) => void
  onAttached: (link: ValidationLink) => void
  onOpenE2e?: () => void
}) {
  const titleId = useId()
  const [attachOpen, setAttachOpen] = useState(false)
  const links = validations.filter((link) => link.suggestion_index === index)
  const latest = latestValidation(validations, index)
  const size = narrow ? 'lg' : 'sm'

  const actions = (
    <div className="eval-ui-ad-card-actions">
      <CopyPlan title={suggestion.title} plan={suggestion.validation} narrow={narrow} />
      {terminal ? (
        <Button variant="pill" size={size} onClick={() => setAttachOpen(true)}>
          <FlaskConical size={16} aria-hidden="true" />
          {links.length > 0 ? 'Replace E2E runs' : 'Attach E2E runs'}
        </Button>
      ) : null}
    </div>
  )

  return (
    <Card className="eval-ui-ad-card" role="article" aria-labelledby={titleId}>
      <div className="eval-ui-ad-card-head">
        <span className="eval-ui-ad-mono-quiet">S{index + 1}</span>
        <Pill>Not validated</Pill>
        {latest && !latest.comparability.comparable ? <Pill tone="alert">Not comparable</Pill> : null}
        {narrow ? null : actions}
      </div>
      <h3 id={titleId} className="eval-ui-ad-card-title">
        {suggestion.title}
      </h3>
      {narrow ? actions : null}
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
      {narrow ? (
        <Disclosure
          className="eval-ui-ad-plan-disclosure"
          summary={(open) => (
            <>
              <PlanHead plan={suggestion.validation} heading={false} />
              {open ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
            </>
          )}
        >
          <div className="eval-ui-ad-plan-body">
            <PlanBody plan={suggestion.validation} />
          </div>
        </Disclosure>
      ) : (
        <CardHighlight className="eval-ui-ad-plan">
          <div className="eval-ui-ad-plan-head">
            <PlanHead plan={suggestion.validation} heading />
          </div>
          <PlanBody plan={suggestion.validation} />
        </CardHighlight>
      )}
      <ValidationPanel links={links} />
      {terminal ? (
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
  assets,
  narrow,
  terminal,
  onJump,
  onAttached,
  onOpenE2e,
}: {
  api: EvalApi
  evaluationId: string
  assets: AnalysisAssets
  narrow: boolean
  terminal: boolean
  onJump: (entry: EntryRef) => void
  onAttached: (link: ValidationLink) => void
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
            index={index}
            suggestion={suggestion}
            snapshot={assets.snapshot}
            validations={assets.validations}
            narrow={narrow}
            terminal={terminal}
            onJump={onJump}
            onAttached={onAttached}
            onOpenE2e={onOpenE2e}
          />
        ))
      )}
      {rejected.length > 0 ? <Rejected items={rejected} /> : null}
    </section>
  )
}
