import {
  Badge,
  type BadgeVariant,
  Button,
  EmptyState,
  Eyebrow,
  type Host,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage, formatDuration, formatRelative } from '@iii-dev/console-ui/format'
import { ArrowLeft } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../api'
import {
  actorStatusLine,
  criterionLine,
  cycleInput,
  decisionView,
  deltaLine,
  findingView,
  formatMetric,
  gaps,
  isActive,
  nextStage,
  outcomeView,
  type Progress,
  promotionSummary,
  sentinelResult,
  stageLabel,
  statusView,
  type Tone,
  validationView,
  visitedStages,
} from '../model'
import type {
  ActorCost,
  Canary,
  CandidateState,
  Coverage,
  Criterion,
  CycleAnalysis,
  CycleDecision,
  CyclePlan,
  CycleRecord,
  CycleResultResponse,
  CycleUsage,
  Evidence,
  EvidenceIdentity,
  EvidenceSide,
  PromotionJournal,
  PromotionPhase,
  ProposalState,
  SentinelForward,
  SentinelResult,
  Triage,
} from '../types'

const BADGE: Record<Tone, BadgeVariant> = {
  neutral: 'default',
  accent: 'accent',
  ok: 'ok',
  warn: 'warn',
  alert: 'alert',
}
const PANEL: Record<Tone, 'info' | 'success' | 'warn' | 'alert'> = {
  neutral: 'info',
  accent: 'info',
  ok: 'success',
  warn: 'warn',
  alert: 'alert',
}
const stamp = (at: number) => new Date(at).toLocaleString()
const usd = (value: number) => `$${value.toFixed(2)}`

export function CycleDetail({
  host,
  api,
  cycleId,
  listed,
  onBack,
  onChanged,
}: {
  host: Host
  api: EvalApi
  cycleId: string
  /** The list's copy: a newer `updated_at` re-reads the result. */
  listed?: CycleRecord
  onBack?: () => void
  onChanged: (announcement: string) => void
}) {
  const [response, setResponse] = useState<CycleResultResponse | null | undefined>(undefined)
  const [error, setError] = useState<string | null>(null)
  const [cancelling, setCancelling] = useState(false)
  const { confirm, dialog } = useConfirm()
  const seq = useRef(0)

  const load = useCallback(async () => {
    const mine = ++seq.current
    try {
      const next = await api.result(cycleId)
      if (mine !== seq.current) return
      setResponse(next)
      setError(null)
    } catch (failure) {
      if (mine === seq.current) setError(errorMessage(failure))
    }
  }, [api, cycleId])

  const version = listed?.updated_at
  useEffect(() => {
    void version
    void load()
  }, [load, version])

  async function cancel() {
    const ok = await confirm({
      title: 'Cancel this cycle?',
      description: 'Work already dispatched keeps its recorded cost; nothing further starts.',
      confirmLabel: 'Cancel cycle',
      cancelLabel: 'Keep running',
      tone: 'danger',
    })
    if (!ok) return
    setCancelling(true)
    try {
      const answer = await api.cancel(cycleId)
      onChanged(answer.cancelled ? 'Cycle cancelled' : `Cycle already ${answer.status}`)
      await load()
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setCancelling(false)
    }
  }

  const back = onBack ? (
    <Button variant="ghost" size="lg" className="eval-ui-back" onClick={onBack}>
      <ArrowLeft size={16} aria-hidden /> Cycles
    </Button>
  ) : null

  if (response === undefined && !error) {
    return (
      <article className="eval-ui-detail" aria-busy="true">
        {back}
        <p className="eval-ui-hint">Reading the cycle…</p>
      </article>
    )
  }
  if (!response) {
    return (
      <article className="eval-ui-detail">
        {back}
        {error ? (
          <StatusPanel variant="alert" headline="Couldn't read the cycle" detail={error} />
        ) : (
          <EmptyState title="Cycle not found" description={`No cycle ${cycleId} is stored.`} />
        )}
      </article>
    )
  }

  const cycle = response.cycle
  const input = cycleInput(cycle)
  const status = statusView(cycle.status)
  const outcome = outcomeView(cycle)
  const found = gaps(cycle)
  const path = visitedStages(cycle)
  const at = path.length - 1

  return (
    <article className="eval-ui-detail" aria-labelledby="eval-ui-detail-title">
      {dialog}
      {back}
      <header className="eval-ui-head">
        <Eyebrow as="p">{input.kind === 'objective' ? 'Objective' : 'E2E execution'}</Eyebrow>
        <h2
          id="eval-ui-detail-title"
          className={input.kind === 'execution' ? 'eval-ui-title eval-ui-mono' : 'eval-ui-title'}
        >
          {input.value}
        </h2>
        <div className="eval-ui-head-meta">
          <Badge variant={BADGE[status.tone]}>{status.label}</Badge>
          <span className="eval-ui-mono eval-ui-faint">{cycle.cycle_id}</span>
          <span className="eval-ui-faint">updated {formatRelative(cycle.updated_at)}</span>
        </div>
      </header>
      {error ? <StatusPanel variant="alert" headline="Last action failed" detail={error} /> : null}

      <Section title="State">
        <ol className="eval-ui-stages" aria-label="Stages so far">
          {path.map((stage, index) => (
            <li
              key={`${stage}-${index}`}
              data-state={index < at ? 'done' : 'current'}
              aria-current={index === at ? 'step' : undefined}
            >
              {stageLabel(stage)}
            </li>
          ))}
        </ol>
        <p>{nextStage(cycle)}</p>
        {cycle.next?.findings?.length ? (
          <p className="eval-ui-faint">
            Rests on findings <span className="eval-ui-mono">{cycle.next.findings.join(', ')}</span>
          </p>
        ) : null}
        {isActive(cycle.status) ? (
          <Button variant="ghost" disabled={cancelling} onClick={() => void cancel()}>
            {cancelling ? 'Cancelling…' : 'Cancel cycle'}
          </Button>
        ) : null}
      </Section>

      {outcome ? (
        <Section title="Outcome">
          <StatusPanel variant={PANEL[outcome.tone]} headline={outcome.label} detail={outcome.reason} />
          {cycle.config ? (
            <p className="eval-ui-faint">
              Frozen configuration <span className="eval-ui-mono">{cycle.config.revision.slice(0, 12)}</span>
            </p>
          ) : (
            <p className="eval-ui-faint">Created unconfigured.</p>
          )}
        </Section>
      ) : null}

      {found.length > 0 ? (
        <Section title="Unknown, no coverage and inconclusive">
          <ul className="eval-ui-gaps">
            {found.map((gap, index) => (
              <li key={`${gap.kind}-${index}`}>
                <Badge variant={gap.kind === 'unknown' ? 'default' : 'warn'}>{gap.kind}</Badge>
                <span className="eval-ui-gap-subject">{gap.subject}</span>
                <span>{gap.reason}</span>
              </li>
            ))}
          </ul>
        </Section>
      ) : null}

      {cycle.coverage ? <CoverageSection coverage={cycle.coverage} /> : null}
      {cycle.triage ? <TriageSection triage={cycle.triage} /> : null}

      {cycle.plan ? (
        <Section title="Frozen plan" note={`Frozen ${stamp(cycle.plan.frozen_at)}, before any dispatch`}>
          <dl className="eval-ui-facts">
            <Fact label="Objective" value={cycle.plan.objective} />
            <Fact label="Criterion" value={criterionLine(cycle.plan.criterion)} />
            {(cycle.plan.preserved ?? []).map((criterion, index) => (
              <Fact key={index} label={`Preserved #${index + 1}`} value={criterionLine(criterion)} />
            ))}
            <Fact label="Controls" value={cycle.plan.controls?.length ? cycle.plan.controls.join(', ') : 'none'} />
            <Fact
              label="Sampling"
              value={`${cycle.plan.sampling.runs_per_side} runs per side${cycle.plan.sampling.seeds?.length ? ` · cases ${cycle.plan.sampling.seeds.join(', ')}` : ''}`}
            />
            <Fact
              label="Subject"
              mono
              value={`${cycle.plan.parameters.provider}/${cycle.plan.parameters.model}${cycle.plan.parameters.agent ? ` · ${cycle.plan.parameters.agent}` : ''}`}
            />
            <Fact
              label="Runs on"
              value={`${cycle.plan.parameters.where}${cycle.plan.parameters.stack ? ` · ${cycle.plan.parameters.stack.name}` : ''} · ${cycle.plan.parameters.runs} runs · ${cycle.plan.parameters.technical_retries} technical retries`}
            />
            <Fact label="Scenarios" mono value={cycle.plan.parameters.scenarios.join(', ')} />
            {cycle.plan.tests.map((test) => (
              <Fact
                key={`${test.scenario_id}:${test.case_id}`}
                label="Frozen test"
                mono
                value={`${test.scenario_id} · ${test.case_id} · seed ${test.seed} · behavior ${test.behavior_sha256.slice(0, 12)} · contract ${test.contract_sha256.slice(0, 12)} · inputs ${test.inputs_sha256.slice(0, 12)}`}
              />
            ))}
            <Fact label="Baseline reserve" value={`$${cycle.plan.estimated_usd.toFixed(2)}`} />
            <Fact label="Policy" mono value={cycle.plan.policy_revision} />
          </dl>
        </Section>
      ) : null}

      {cycle.executions?.length ? (
        <Section title="Executions">
          <ul className="eval-ui-rows">
            {cycle.executions.map((execution, index) => (
              <li key={execution.label ?? execution.execution_id ?? index}>
                <Badge variant={execution.state === 'lost' ? 'alert' : 'default'}>{execution.role}</Badge>
                <span>{execution.state}</span>
                <span className="eval-ui-mono">{execution.execution_id ?? 'id not known yet'}</span>
                {execution.label ? <span className="eval-ui-mono eval-ui-faint">{execution.label}</span> : null}
                {execution.detail ? <span className="eval-ui-faint">{execution.detail}</span> : null}
              </li>
            ))}
          </ul>
        </Section>
      ) : null}

      {cycle.analyses?.length ? (
        <Section title="Findings" note="Deterministic first pass over each imported execution">
          {cycle.analyses.map((analysis) => (
            <AnalysisBlock key={analysis.admission_key} host={host} analysis={analysis} />
          ))}
        </Section>
      ) : null}

      {cycle.baseline ? (
        <Section title="Baseline" note={cycle.baseline.usable ? 'Usable for comparison' : 'Not usable'}>
          <p>{cycle.baseline.reason}</p>
          <BaselineTable
            rows={[
              { label: 'Primary', criterion: cycle.plan?.criterion, side: cycle.baseline.primary },
              ...cycle.baseline.preserved.map((side, index) => ({
                label: `Preserved #${index + 1}`,
                criterion: cycle.plan?.preserved?.[index],
                side,
              })),
            ]}
          />
        </Section>
      ) : null}

      {cycle.proposal ? <ProposalSection state={cycle.proposal} /> : null}
      {cycle.candidate ? <CandidateSection state={cycle.candidate} /> : null}
      {cycle.decision ? <DecisionMetrics title="Source decision" decision={cycle.decision} plan={cycle.plan} /> : null}
      {cycle.artifact_decision ? (
        <DecisionMetrics title="Artifact decision (rc)" decision={cycle.artifact_decision} plan={cycle.plan} />
      ) : null}
      {cycle.promotion || cycle.canary || cycle.stage === 'promotion' ? (
        <PromotionSection journal={cycle.promotion} canary={cycle.canary} />
      ) : null}

      <Section title="Spend">
        {response.usage ? (
          <UsageFacts usage={response.usage} />
        ) : (
          <p className="eval-ui-faint">No work reserved yet: nothing spent.</p>
        )}
      </Section>

      <Section title="History">
        <ol className="eval-ui-rows">
          {cycle.history.map((event, index) => (
            <li key={`${event.at}-${index}`}>
              <span className="eval-ui-faint eval-ui-nums">{stamp(event.at)}</span>
              <span>
                {statusView(event.status).label} · {stageLabel(event.stage)}
              </span>
              <span className="eval-ui-faint">{event.detail}</span>
            </li>
          ))}
        </ol>
      </Section>
    </article>
  )
}

function CoverageSection({ coverage }: { coverage: Coverage }) {
  return (
    <Section
      title="Coverage"
      note={`${coverage.covered ? 'Covered' : 'Not covered'} · ${coverage.selected.length} of ${coverage.tests_total} tests selected ${stamp(coverage.selected_at)}`}
    >
      <dl className="eval-ui-facts">
        <Fact label="Need" mono value={coverage.need} />
        <Fact label="Catalog" mono value={`${coverage.tests_revision} · ${coverage.catalog_sha256.slice(0, 12)}`} />
        {coverage.covered && coverage.gaps.length ? (
          <Fact label="Limitations" value={coverage.gaps.join('; ')} />
        ) : null}
      </dl>
      {coverage.selected.map((test) => (
        <div key={`${test.scenario_id}:${test.case_id}`} className="eval-ui-card">
          <p className="eval-ui-mono">
            {test.scenario_id} · {test.case_id} · seed {test.seed}
          </p>
          <p>{test.rationale}</p>
          {test.history_note ? <p className="eval-ui-faint">{test.history_note}</p> : null}
          <dl className="eval-ui-facts">
            {test.matched_terms.length ? <Fact label="Matched" value={test.matched_terms.join(', ')} /> : null}
            {test.invariants.length ? <Fact label="Invariants" value={test.invariants.join('; ')} /> : null}
            <Fact
              label="Identity"
              mono
              value={`behavior ${test.behavior_sha256.slice(0, 12)} · contract ${test.contract_sha256.slice(0, 12)} · inputs ${test.inputs_sha256.slice(0, 12)}`}
            />
          </dl>
          {test.criteria.length ? (
            <ul className="eval-ui-rows" aria-label={`Criteria of ${test.scenario_id}`}>
              {test.criteria.map((criterion) => (
                <li key={criterion.id}>
                  <Badge variant={criterion.gate ? 'warn' : 'default'}>{criterion.gate ? 'gate' : 'score'}</Badge>
                  <span className="eval-ui-mono">{criterion.id}</span>
                  <span>{criterion.description}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ))}
    </Section>
  )
}

const IDENTITY: (keyof Omit<EvidenceIdentity, 'scenarios'>)[] = [
  'provider',
  'model',
  'harness_version',
  'harness_binary_sha256',
  'workers_revision',
  'engine_version',
  'e2e_revision',
  'contract_digest',
  'stack_digest',
  'fixed_stack_digest',
  'subject_sha256',
  'artifacts_digest',
]

function AnalysisBlock({ host, analysis }: { host: Host; analysis: CycleAnalysis }) {
  return (
    <div className="eval-ui-card">
      <p>
        <Badge>{analysis.role}</Badge> <span className="eval-ui-mono">{analysis.execution_id}</span>{' '}
        <span className="eval-ui-faint">analyzed {stamp(analysis.analyzed_at)}</span>
      </p>
      {analysis.findings.length ? (
        <ul className="eval-ui-rows">
          {analysis.findings.map((finding) => {
            const view = findingView(finding.class)
            return (
              <li key={finding.fingerprint}>
                <Badge variant={BADGE[view.tone]}>{view.label}</Badge>
                <span className="eval-ui-mono">{finding.signal}</span>
                <span>{finding.observation}</span>
              </li>
            )
          })}
        </ul>
      ) : (
        <p className="eval-ui-faint">No findings.</p>
      )}
      <dl className="eval-ui-facts">
        {IDENTITY.map((field) => (
          <Fact key={field} label={field} mono value={analysis.identity[field] ?? 'unknown'} />
        ))}
      </dl>
      {analysis.sentinel ? <SentinelBlock host={host} forward={analysis.sentinel} /> : null}
    </div>
  )
}

const reported = (value: number | undefined) => (value === undefined ? 'not reported' : String(value))

/** Sentinel's own answer for the runs' failures: separate from the loop's findings and decision. */
function SentinelBlock({ host, forward }: { host: Host; forward: SentinelForward }) {
  const results = (forward.results ?? []).map(sentinelResult)
  const unreadable = results.filter((result) => result === null).length
  return (
    <section className="eval-ui-sentinel" aria-label="Sentinel triage">
      <h4 className="eval-ui-subhead">Sentinel triage</h4>
      <p className="eval-ui-faint">
        Sentinel's classification of the recorded failures, reused as it answered; not part of the computed outcome.
      </p>
      <p className="eval-ui-nums">
        {forward.observations} forwarded · {reported(forward.recorded)} recorded · {reported(forward.deduped)} deduped ·{' '}
        {reported(forward.skipped)} skipped
      </p>
      {forward.error ? <StatusPanel variant="warn" headline="Sentinel did not answer" detail={forward.error} /> : null}
      {results.length ? (
        <ul className="eval-ui-rows">
          {results.map((result) =>
            result ? <SentinelRow key={result.dedupe_key} host={host} result={result} /> : null,
          )}
        </ul>
      ) : null}
      {unreadable ? <p className="eval-ui-faint">{unreadable} result(s) were not in Sentinel's result shape.</p> : null}
    </section>
  )
}

function SentinelRow({ host, result }: { host: Host; result: SentinelResult }) {
  const triage = result.triage
  const groupId = result.group_id
  const tokens = triage?.usage
  const triageLine = triage
    ? [
        triage.kind,
        triage.confidence === undefined ? null : `confidence ${triage.confidence.toFixed(2)}`,
        triage.source ? `by ${triage.source}` : null,
        triage.model ?? null,
        tokens && (tokens.input_tokens !== undefined || tokens.output_tokens !== undefined)
          ? `${reported(tokens.input_tokens)} in / ${reported(tokens.output_tokens)} out tokens`
          : null,
        triage.question ? `asked: ${triage.question}` : null,
        triage.criterion ? `criterion: ${triage.criterion}` : null,
      ]
        .filter(Boolean)
        .join(' · ')
    : null
  return (
    <li className="eval-ui-sentinel-row">
      <span className="eval-ui-row-line">
        <Badge>{result.classification}</Badge>
        <span>{result.outcome}</span>
        {groupId && host.panels ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => host.panels?.open({ pageId: 'sentinel', context: { group_id: groupId } })}
          >
            Group <span className="eval-ui-mono">{groupId}</span>
          </Button>
        ) : groupId ? (
          <span className="eval-ui-mono">group {groupId}</span>
        ) : null}
        {result.group_status ? <span className="eval-ui-faint">{result.group_status}</span> : null}
        {result.occurrence_id ? (
          <span className="eval-ui-mono eval-ui-faint">occurrence {result.occurrence_id}</span>
        ) : null}
      </span>
      <span className="eval-ui-faint">{triageLine ?? 'Not triaged by Sentinel yet.'}</span>
    </li>
  )
}

const PROGRESS: Record<Progress, { label: string; variant: BadgeVariant }> = {
  not_requested: { label: 'not requested', variant: 'default' },
  requested: { label: 'requested, not confirmed', variant: 'accent' },
  confirmed: { label: 'confirmed', variant: 'ok' },
  unknown: { label: 'unknown', variant: 'warn' },
}

const json = (value: unknown) => JSON.stringify(value, null, 2)
const money = (value: number | null) => (value === null ? 'unknown' : `$${value.toFixed(4)}`)
const tokens = (value: number | null) => (value === null ? 'unknown' : String(value))

function ActorCostFact({ cost }: { cost: ActorCost }) {
  return (
    <Fact
      label="Cost"
      value={`${money(cost.cost_usd)} · ${tokens(cost.input_tokens)} in / ${tokens(cost.output_tokens)} out tokens`}
    />
  )
}

/** Why the cycle proceeds past its findings. A JEV answer is shown only when recorded; never the decision. */
function TriageSection({ triage }: { triage: Triage }) {
  const jev = triage.source === 'jev'
  return (
    <Section
      title="Triage"
      note={jev ? 'Semantic judgment (JEV), separate from the computed decision' : `By ${triage.source}`}
    >
      <dl className="eval-ui-facts">
        <Fact label="Source" value={triage.source} />
        <Fact label="Outcome" value={triage.outcome.replace('_', ' ')} />
        <Fact label="Reason" value={triage.reason} />
        {triage.findings.length ? <Fact label="Findings" mono value={triage.findings.join(', ')} /> : null}
        {triage.request_id ? <Fact label="Request" mono value={triage.request_id} /> : null}
        {triage.model ? <Fact label="Model" mono value={triage.model} /> : null}
        {triage.evidence_digest ? <Fact label="Evidence digest" mono value={triage.evidence_digest} /> : null}
      </dl>
      {jev && triage.answer === undefined ? <p className="eval-ui-faint">No JEV answer recorded.</p> : null}
      {triage.question !== undefined ? <JsonBlock label="Question" value={triage.question} /> : null}
      {triage.answer !== undefined ? <JsonBlock label="Answer" value={triage.answer} /> : null}
      {triage.usage !== undefined ? <JsonBlock label="Usage" value={triage.usage} /> : null}
      {triage.reused?.length ? <JsonBlock label="Sentinel triage reused" value={triage.reused} /> : null}
    </Section>
  )
}

function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <details className="eval-ui-json">
      <summary>{label}</summary>
      <pre>{json(value)}</pre>
    </details>
  )
}

function ProposalSection({ state }: { state: ProposalState }) {
  const proposal = state.proposal
  return (
    <Section title="Proposal" note="Read-only, bounded; turns evidence into a brief">
      <dl className="eval-ui-facts">
        <Fact label="Proposal" mono value={state.identity.proposal_id} />
        <Fact label="Base" mono value={state.identity.base_sha} />
        <Fact label="Session" mono value={state.identity.session_id} />
        {proposal ? (
          <>
            <Fact label="Status" value={actorStatusLine(proposal.status)} />
            <Fact label="Model" mono value={`${proposal.provider}/${proposal.model}`} />
            <ActorCostFact cost={proposal.cost} />
            {proposal.hypothesis ? <Fact label="Hypothesis" value={proposal.hypothesis} /> : null}
            {proposal.change ? <Fact label="Change" value={proposal.change} /> : null}
            {proposal.input.criteria.length ? (
              <Fact label="Criteria" value={proposal.input.criteria.join('; ')} />
            ) : null}
            {proposal.input.case_ids.length ? (
              <Fact label="Cases" mono value={proposal.input.case_ids.join(', ')} />
            ) : null}
          </>
        ) : (
          <Fact label="Status" value="identity recorded; no state yet" />
        )}
      </dl>
      {proposal?.citations.length ? (
        <ul className="eval-ui-rows" aria-label="Citations">
          {proposal.citations.map((citation) => (
            <li key={`${citation.path}:${citation.line_start}`} className="eval-ui-mono">
              {citation.path}:{citation.line_start}-{citation.line_end}
            </li>
          ))}
        </ul>
      ) : null}
    </Section>
  )
}

function CandidateSection({ state }: { state: CandidateState }) {
  const candidate = state.candidate
  const commit = candidate?.commit
  return (
    <Section title="Candidate" note="Isolated actor in its own worktree">
      <dl className="eval-ui-facts">
        <Fact label="Candidate" mono value={state.identity.candidate_id} />
        <Fact label="Branch" mono value={state.identity.branch} />
        <Fact label="Base" mono value={state.identity.base_sha} />
        <Fact label="Session" mono value={state.identity.session_id} />
        {candidate ? (
          <>
            <Fact label="Status" value={actorStatusLine(candidate.status)} />
            <ActorCostFact cost={candidate.cost} />
          </>
        ) : (
          <Fact label="Status" value="identity recorded; no state yet" />
        )}
        {commit ? <Fact label="Commit" mono value={commit.candidate_sha} /> : null}
        {commit ? <Fact label="Patch digest" mono value={commit.patch_digest} /> : null}
        {state.candidate_tree ? <Fact label="Tree" mono value={state.candidate_tree} /> : null}
        <Fact
          label="Published"
          mono
          value={
            state.publish
              ? `${state.publish.remote}/${state.publish.branch} · ${state.publish.builder_ref}`
              : 'not published'
          }
        />
      </dl>
      {commit?.paths.length ? (
        <ul className="eval-ui-rows" aria-label="Changed paths">
          {commit.paths.map((path) => (
            <li key={path.path}>
              <Badge>{path.change}</Badge>
              <span className="eval-ui-mono">{path.path}</span>
            </li>
          ))}
        </ul>
      ) : null}
    </Section>
  )
}

function DecisionMetrics({ title, decision, plan }: { title: string; decision: CycleDecision; plan?: CyclePlan }) {
  const view = decisionView(decision.outcome)
  return (
    <Section title={title} note="Computed in code from the frozen plan">
      <p>
        <Badge variant={BADGE[view.tone]}>{view.label}</Badge> {decision.reason}
      </p>
      <dl className="eval-ui-facts">
        <Fact label="Baseline" mono value={decision.baseline_execution_id} />
        <Fact label="Candidate" mono value={decision.candidate_execution_id} />
        <Fact label="Policy" mono value={decision.policy_revision} />
        <Fact label="Decided" value={stamp(decision.decided_at)} />
      </dl>
      <EvidenceTable
        rows={[
          { label: 'Primary', criterion: plan?.criterion, evidence: decision.primary },
          ...decision.preserved.map((evidence, index) => ({
            label: `Preserved #${index + 1}`,
            criterion: plan?.preserved?.[index],
            evidence,
          })),
        ]}
      />
      <h4 className="eval-ui-subhead">Comparability</h4>
      <ul className="eval-ui-checks">
        {decision.comparability.checks.map((check) => (
          <li key={check.field} data-ok={check.matches ? 'true' : 'false'}>
            <span>{check.matches ? 'match' : 'differs'}</span>
            <span className="eval-ui-mono">{check.field}</span>
            <span className="eval-ui-mono eval-ui-faint">
              {check.baseline ?? 'unknown'} → {check.candidate ?? 'unknown'}
            </span>
          </li>
        ))}
      </ul>
    </Section>
  )
}

/** One journal entry in words, with the evidence that phase carries. */
function phaseLine(entry: PromotionPhase): string {
  switch (entry.phase) {
    case 'source_approved':
      return `source ${entry.outcome} · candidate ${entry.candidate_sha.slice(0, 12)} on ${entry.base_sha.slice(0, 12)}`
    case 'pull_request':
      return `PR #${entry.number} ${entry.created ? 'opened' : 'found'} · head ${entry.head_sha.slice(0, 12)} · ${entry.url}`
    case 'merge_requested':
      return `PR #${entry.pull_request} · merge sha ${entry.merge_sha ?? 'not returned'}`
    case 'merged':
      return `PR #${entry.pull_request} · merged ${entry.merged_sha.slice(0, 12)} · tree ${entry.merged_tree === entry.candidate_tree ? 'matches' : 'differs from'} the approved tree`
    case 'rc_planned':
    case 'finalize_planned':
      return `${entry.kind} plan ${entry.plan_id} → ${entry.target_version} (${entry.channel}), expires ${entry.expires_at}`
    case 'rc_confirmed':
    case 'finalize_confirmed':
      return `plan ${entry.plan.plan_id} confirmed · operation ${entry.operation_id}`
    case 'rc_published':
    case 'stable_published':
      return `${entry.version} · archive ${entry.archive_sha256.slice(0, 12)} · from ${entry.source_sha.slice(0, 12)}`
    case 'artifact_approved':
      return `artifact ${entry.outcome} · ${entry.version} · binary ${entry.binary_sha256 ? entry.binary_sha256.slice(0, 12) : 'unknown'}`
    case 'rollback_destination':
      return `${entry.version} · binary ${entry.binary_sha256.slice(0, 12)}`
    case 'activation_requested':
    case 'rollback_requested':
      return `${entry.version} · operation ${entry.operation_id} · ${entry.accepted ? 'accepted by Compose' : 'not accepted'}`
    case 'activation_confirmed':
    case 'rollback_confirmed':
      return `${entry.version} running · pid ${entry.pid} · binary ${entry.binary_sha256.slice(0, 12)} · ${entry.namespace}`
    case 'stopped':
      return `${entry.step} · ${entry.kind}: ${entry.reason}`
  }
}

function PromotionSection({ journal, canary }: { journal?: PromotionJournal; canary?: Canary }) {
  const summary = promotionSummary(journal)
  return (
    <Section title="Promotion" note="Requested and confirmed are separate journal facts">
      <dl className="eval-ui-facts">
        {(
          [
            ['Merge', summary.merge],
            ['Release', summary.release],
            ['Activation', summary.activation],
            ['Rollback', summary.rollback],
          ] as const
        ).map(([label, progress]) => (
          <Fact
            key={label}
            label={label}
            value={
              <Badge variant={PROGRESS[progress].variant}>
                {label === 'Rollback' && progress === 'confirmed' ? 'reverted (confirmed)' : PROGRESS[progress].label}
              </Badge>
            }
          />
        ))}
        {canary ? (
          <Fact
            label="Canary"
            value={
              <>
                <Badge variant={canary.outcome === 'held' ? 'ok' : canary.outcome === 'violated' ? 'alert' : 'warn'}>
                  {canary.outcome}
                </Badge>{' '}
                <span className="eval-ui-mono">{canary.execution_id}</span> · {canary.reason}
              </>
            }
          />
        ) : null}
      </dl>
      {journal?.phases.length ? (
        <ol className="eval-ui-rows" aria-label={`Promotion journal ${journal.promotion_id}`}>
          {journal.phases.map((entry, index) => (
            <li key={`${entry.phase}-${index}`}>
              <Badge variant={entry.phase === 'stopped' ? 'warn' : 'default'}>{entry.phase.replaceAll('_', ' ')}</Badge>
              <span className="eval-ui-mono eval-ui-faint">{phaseLine(entry)}</span>
            </li>
          ))}
        </ol>
      ) : (
        <p className="eval-ui-faint">No promotion phase journaled.</p>
      )}
    </Section>
  )
}

function UsageFacts({ usage }: { usage: CycleUsage }) {
  const { budget } = usage
  return (
    <dl className="eval-ui-facts">
      <Fact label="Spend" value={`${usd(usage.spend_usd)} of ${usd(budget.max_spend_usd)}`} />
      <Fact
        label="Unknown cost"
        value={
          usage.unknown_cost ? `${usage.unknown_cost} work item(s) reported no cost; charged at their estimate` : 'none'
        }
      />
      <Fact label="Calls" value={`${usage.calls} of ${budget.max_calls}`} />
      <Fact label="Candidates" value={`${usage.candidates} of ${budget.max_candidates}`} />
      <Fact
        label="Elapsed"
        value={`${formatDuration(usage.elapsed_ms)} of ${formatDuration(budget.max_duration_ms)}`}
      />
    </dl>
  )
}

function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section className="eval-ui-section" aria-label={title}>
      <header className="eval-ui-section-head">
        <h3>{title}</h3>
        {note ? <span className="eval-ui-faint">{note}</span> : null}
      </header>
      {children}
    </section>
  )
}

function Fact({ label, value, mono }: { label: string; value: ReactNode; mono?: boolean }) {
  return (
    <div className="eval-ui-fact">
      <dt>{label}</dt>
      <dd className={mono ? 'eval-ui-mono' : undefined}>{value}</dd>
    </div>
  )
}

function BaselineTable({ rows }: { rows: { label: string; criterion?: Criterion; side: EvidenceSide }[] }) {
  return (
    <div className="eval-ui-table-scroll">
      <table className="eval-ui-table eval-ui-table-narrow">
        <thead>
          <tr>
            <th scope="col">Measure</th>
            <th scope="col">Mean</th>
            <th scope="col">Measured runs</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ label, criterion, side }) => (
            <tr key={label}>
              <th scope="row">
                {label}
                {criterion ? <span className="eval-ui-faint">{criterion.metric}</span> : null}
              </th>
              <td>{criterion ? formatMetric(criterion.metric, side.mean) : '—'}</td>
              <td>
                {side.n} of {side.runs.length}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function EvidenceTable({ rows }: { rows: { label: string; criterion?: Criterion; evidence: Evidence }[] }) {
  return (
    <div className="eval-ui-table-scroll">
      <table className="eval-ui-table">
        <thead>
          <tr>
            <th scope="col">Measure</th>
            <th scope="col">Baseline</th>
            <th scope="col">Candidate</th>
            <th scope="col">Delta</th>
            <th scope="col">Outcome</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ label, criterion, evidence }) => {
            const metric = criterion?.metric
            const view = validationView(evidence.computed_outcome)
            return (
              <tr key={label}>
                <th scope="row">
                  {label}
                  <span className="eval-ui-faint eval-ui-mono">{evidence.scenario_id}</span>
                </th>
                <td>
                  {metric ? formatMetric(metric, evidence.baseline.mean) : '—'}
                  <span className="eval-ui-faint">n={evidence.baseline.n}</span>
                </td>
                <td>
                  {metric ? formatMetric(metric, evidence.candidate.mean) : '—'}
                  <span className="eval-ui-faint">n={evidence.candidate.n}</span>
                </td>
                <td className="eval-ui-mono">{metric ? deltaLine(metric, evidence) : 'unknown'}</td>
                <td>
                  <Badge variant={BADGE[view.tone]}>{view.label}</Badge>
                  <span className="eval-ui-faint">{evidence.reason}</span>
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
