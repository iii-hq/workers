import {
  Badge,
  Breadcrumb,
  Button,
  Chip,
  EmptyState,
  Eyebrow,
  type Host,
  IconButton,
  JsonHighlight,
  PageBody,
  PageHeader,
  PageMain,
  Panel,
  PanelBody,
  PanelHeader,
  Skeleton,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  Toolbar,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, Info, MessageSquare, RefreshCcw } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../api'
import {
  badge,
  canRetry,
  criterionName,
  cycleInput,
  cycleTitle,
  decisionView,
  deltaLine,
  FIX,
  findingView,
  formatEffect,
  formatMetric,
  gaps,
  isActive,
  type Lost,
  lostCriteria,
  type Mark,
  marks,
  nowView,
  PHASES,
  phaseOf,
  plural,
  resultHeadline,
  review,
  runningExecution,
  runValue,
  sentinelResult,
  shortId,
  span,
  type Tone,
} from '../model'
import type {
  Criterion,
  CycleDecision,
  CycleRecord,
  CycleResultResponse,
  CycleUsage,
  Evidence,
  LoopConfig,
} from '../types'
import { ago, Caret, MarkIcon, PANEL, StatusChip, ToneIcon, usd } from './bits'
import type { Nav } from './index'

const E2E_PAGE = 'harness-e2e'
const SENTINEL_PAGE = 'sentinel'
const ACTIVITY_SHOWN = 7
const clock = (at: number) => new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false })
/** The detail as a sentence. */
const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)
const short = (sha: string) => sha.slice(0, 7)

export function CycleDetail({
  host,
  api,
  cycleId,
  listed,
  current,
  narrow,
  nav,
  onClose,
  onChanged,
}: {
  host: Host
  api: EvalApi
  cycleId: string
  /** The list's copy: a newer `updated_at` re-reads the result. */
  listed?: CycleRecord
  current: LoopConfig | null
  narrow: boolean
  nav: Nav
  onClose?: () => void
  onChanged: () => void
}) {
  const [response, setResponse] = useState<CycleResultResponse | null | undefined>(undefined)
  const [error, setError] = useState<string | null>(null)
  const [cancelling, setCancelling] = useState(false)
  const [retrying, setRetrying] = useState(false)
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
      nav.announce(answer.cancelled ? 'Cycle cancelled' : `Cycle already ${answer.status}`)
      onChanged()
      await load()
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setCancelling(false)
    }
  }

  async function retry() {
    const runs = current?.runs_per_side
    const estimate = response?.cycle.plan?.estimated_usd
    const ok = await confirm({
      title: 'Measure this again?',
      description: `A new cycle reuses this finding and runs a new baseline${runs ? ` of ${plural(runs, 'run')}` : ''}${
        estimate ? `, about ${usd(estimate)}` : ''
      }. It goes on to propose a change only if the new runs still lose the points.`,
      confirmLabel: 'Try again',
      cancelLabel: 'Not now',
    })
    if (!ok) return
    setRetrying(true)
    try {
      const started = await api.retry(cycleId)
      onChanged()
      nav.announce('Cycle started again')
      nav.cycle(started.cycle_id)
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setRetrying(false)
    }
  }

  const cycle = response?.cycle
  const title = cycle ? cycleTitle(cycle) : cycleId
  const compose = host.chat?.compose

  return (
    <>
      {dialog}
      <PageHeader
        icon={<RefreshCcw size={16} aria-hidden />}
        title="Eval loop"
        description={<span className="eval-ui-mono">{cycleId}</span>}
        onClose={onClose}
      />
      <PageBody>
        <PageMain className="eval-ui-main">
          <Toolbar
            aria-label="Cycle"
            end={
              cycle ? (
                <>
                  {compose ? (
                    <IconButton
                      label="Reference in chat"
                      onClick={() => compose({ text: `Eval cycle ${cycle.cycle_id} (${title}) ` })}
                    >
                      <MessageSquare size={16} aria-hidden />
                    </IconButton>
                  ) : null}
                  {isActive(cycle.status) ? (
                    <Button variant="ghost" disabled={cancelling} onClick={() => void cancel()}>
                      {cancelling ? 'Cancelling…' : 'Cancel cycle'}
                    </Button>
                  ) : null}
                </>
              ) : null
            }
          >
            <IconButton label="Back to cycles" onClick={nav.home}>
              <ArrowLeft size={16} aria-hidden />
            </IconButton>
            {/* A phone has the back button: its path is the cycle alone. */}
            <Breadcrumb
              items={narrow ? [{ label: title }] : [{ label: 'Cycles', onClick: nav.home }, { label: title }]}
            />
          </Toolbar>
          <div className="eval-ui-scroll">
            {response === undefined && !error ? (
              <div className="eval-ui-page" role="status" aria-busy="true" aria-label="Reading the cycle">
                <div className="eval-ui-col">
                  <Skeleton className="eval-ui-sk-title" />
                  <Skeleton className="eval-ui-sk-block" />
                  <Skeleton className="eval-ui-sk-block" />
                </div>
              </div>
            ) : !cycle ? (
              <div className="eval-ui-page">
                {error ? (
                  <StatusPanel
                    variant="alert"
                    headline="Couldn't read the cycle"
                    detail={error}
                    action={
                      <Button variant="ghost" size="sm" onClick={() => void load()}>
                        Retry
                      </Button>
                    }
                  />
                ) : (
                  <EmptyState
                    title="Cycle not found"
                    description={`No cycle ${cycleId} is stored.`}
                    action={{ label: 'Back to cycles', onClick: nav.home }}
                  />
                )}
              </div>
            ) : (
              <Cycle
                host={host}
                cycle={cycle}
                usage={response?.usage ?? null}
                current={current}
                narrow={narrow}
                nav={nav}
                error={error}
                onRetry={canRetry(cycle) ? () => void retry() : undefined}
                retrying={retrying}
              />
            )}
          </div>
        </PageMain>
      </PageBody>
    </>
  )
}

function Cycle({
  host,
  cycle,
  usage,
  current,
  narrow,
  nav,
  error,
  onRetry,
  retrying,
}: {
  host: Host
  cycle: CycleRecord
  usage: CycleUsage | null
  current: LoopConfig | null
  narrow: boolean
  nav: Nav
  error: string | null
  onRetry?: () => void
  retrying: boolean
}) {
  const input = cycleInput(cycle)
  const view = badge(cycle)
  const now = nowView(cycle, current)
  const at = phaseOf(cycle)
  const phase = PHASES[at]
  const mark = marks(cycle)[at]
  const where = isActive(cycle.status)
    ? phase
    : mark === 'done'
      ? at === 3
        ? 'Every phase done'
        : `Ended at ${phase}`
      : `Stopped at ${phase}`
  const running = runningExecution(cycle)
  const openE2E =
    host.panels && running?.execution_id ? () => host.panels?.open({ pageId: E2E_PAGE, context: {} }) : null
  const found = gaps(cycle).filter((gap) => !gap.subject.startsWith('Outcome'))
  const ended = !isActive(cycle.status)
  const lost = lostCriteria(cycle)
  // The runs a person reads: the baseline's when it ran, else the source's.
  const runsId = ['baseline', 'source']
    .map((role) => cycle.executions?.filter((entry) => entry.role === role && entry.execution_id).at(-1))
    .find(Boolean)?.execution_id
  const openRuns =
    ended && host.panels && runsId
      ? () => {
          // The E2E page routes itself from this hash when it opens.
          window.location.hash = `#/ext/${E2E_PAGE}/execution/${runsId}`
          host.panels?.open({ pageId: E2E_PAGE, context: {} })
        }
      : undefined
  const action = now.fix ? (
    <Button variant="pill" size={narrow ? 'lg' : 'md'} onClick={() => now.fix && nav.settings(now.fix)}>
      {FIX[now.fix].action}
    </Button>
  ) : openE2E ? (
    <Button variant="pill" size={narrow ? 'lg' : 'md'} onClick={openE2E}>
      Open in E2E
    </Button>
  ) : null

  return (
    <div className="eval-ui-page">
      <div className="eval-ui-col">
        <header className="eval-ui-head">
          <Eyebrow as="p">
            {input.kind === 'objective' ? (
              'Objective'
            ) : (
              <>
                From E2E execution{' '}
                <span className="eval-ui-mono eval-ui-keep" title={input.value}>
                  {shortId(input.value)}
                </span>
              </>
            )}
          </Eyebrow>
          <h2 className="eval-ui-title">{cycleTitle(cycle)}</h2>
          {cycle.retry_of ? (
            <p className="eval-ui-faint">
              Measured again from{' '}
              <button
                type="button"
                className="eval-ui-link"
                onClick={() => cycle.retry_of && nav.cycle(cycle.retry_of)}
              >
                {cycle.retry_of.slice(0, 14)}
              </button>
            </p>
          ) : null}
          <div className="eval-ui-meta">
            <StatusChip tone={view.tone} label={view.label} pulse={isActive(cycle.status)} />
            <span>{where}</span>
            <span className="eval-ui-num eval-ui-ghost">
              {cycle.completed_at
                ? `finished ${ago(cycle.completed_at)} · took ${span(cycle.completed_at - cycle.created_at)}`
                : `updated ${ago(cycle.updated_at)}`}
            </span>
          </div>
        </header>
        {error ? <StatusPanel variant="alert" headline="Last action failed" detail={error} /> : null}
        {ended && !now.fix ? (
          <Result cycle={cycle} lost={lost} onRetry={onRetry} retrying={retrying} onOpenRuns={openRuns} />
        ) : (
          <>
            <StatusPanel
              variant={PANEL[now.tone]}
              icon={<ToneIcon tone={now.tone} />}
              headline={now.headline}
              detail={now.detail}
              action={narrow ? undefined : action}
            />
            {/* A phone keeps the sentence whole and puts its action underneath, full width. */}
            {narrow && action ? <div className="eval-ui-wide">{action}</div> : null}
          </>
        )}
        {narrow ? <Budget usage={usage} /> : null}
        <Path cycle={cycle} current={current} narrow={narrow} />
        {/* The rule matters once a change can be judged; an ended cycle that never got one keeps it in the record. */}
        {cycle.plan && !cycle.decision && (!ended || cycle.candidate) ? <Rule cycle={cycle} /> : null}
        {cycle.decision ? <Decision title="Source decision" cycle={cycle} decision={cycle.decision} /> : null}
        <Findings cycle={cycle} collapsed={ended && lost.length > 0} />
        {found.length ? <Unknowns host={host} cycle={cycle} found={found} /> : null}
        <Activity cycle={cycle} collapsed={ended} />
        <FullRecord cycle={cycle} />
      </div>
      <aside className="eval-ui-aside" aria-label="Cycle facts">
        {narrow ? null : <Budget usage={usage} />}
        <Details cycle={cycle} />
      </aside>
    </div>
  )
}

// ── what an ended cycle found, and what can follow ──────────────────────

function Result({
  cycle,
  lost,
  onRetry,
  retrying,
  onOpenRuns,
}: {
  cycle: CycleRecord
  lost: Lost[]
  onRetry?: () => void
  retrying: boolean
  onOpenRuns?: () => void
}) {
  const view = badge(cycle)
  // A decision says why below, in its own panel.
  const why = cycle.decision ? '' : view.reason
  return (
    <Panel>
      <PanelHeader>Result</PanelHeader>
      <PanelBody className="eval-ui-stack">
        <p className="eval-ui-lead eval-ui-strong">{resultHeadline(cycle, lost.length)}</p>
        {lost.length ? (
          <div className="eval-ui-stack-sm">
            <Eyebrow as="p">What the runs lost</Eyebrow>
            <ul className="eval-ui-lost">
              {lost.map((item) => (
                <li key={`${item.scenario}:${item.criterion}`}>
                  <p>
                    <span className="eval-ui-mono">{item.criterion}</span> on{' '}
                    <span className="eval-ui-mono">{item.scenario}</span>
                    <span className="eval-ui-num eval-ui-faint">
                      {' · '}lost in {item.lost} of {plural(item.runs, 'run')}
                    </span>
                  </p>
                  {item.description ? <p className="eval-ui-faint">{item.description}</p> : null}
                  {item.failed.length ? (
                    <p className="eval-ui-faint">
                      Check that failed: <span className="eval-ui-mono">{item.failed.join(', ')}</span>
                    </p>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        {why ? (
          <div className="eval-ui-stack-sm">
            <Eyebrow as="p">{cycle.outcome?.code === 'proposed' ? 'What happened' : 'Why it stopped'}</Eyebrow>
            <p className="eval-ui-lead">{sentence(why)}</p>
          </div>
        ) : null}
        {onRetry || onOpenRuns ? (
          <div className="eval-ui-result-actions">
            {onRetry ? (
              <Button variant="primary" disabled={retrying} onClick={onRetry}>
                {retrying ? 'Starting…' : 'Try again'}
              </Button>
            ) : null}
            {onOpenRuns ? (
              <Button variant="ghost" onClick={onOpenRuns}>
                Open the runs in E2E
              </Button>
            ) : null}
          </div>
        ) : null}
        {onRetry ? (
          <p className="eval-ui-faint">
            Try again measures the same test anew; a change is proposed only if the new runs still lose the points.
          </p>
        ) : null}
      </PanelBody>
    </Panel>
  )
}

// ── where the cycle is ──────────────────────────────────────────────────

interface Step {
  mark: Mark
  label: string
  fact?: ReactNode
}

const OUTCOME_MARK = { accepted: 'done', rejected: 'fail', inconclusive: 'stop' } as const

/** What each phase recorded, step by step, in the order it happens. */
function steps(cycle: CycleRecord, current: LoopConfig | null): Step[][] {
  const frozen = cycle.config?.config
  const active = isActive(cycle.status)
  // A setting that was off shows where the cycle got to, or would go next.
  const reached = (phase: number) => active || phaseOf(cycle) >= phase
  const source = cycle.analyses?.find((analysis) => analysis.role === 'source')
  const entry = (role: string) => cycle.executions?.filter((execution) => execution.role === role).at(-1)
  const executionStep = (role: string, label: string): Step | null => {
    const execution = entry(role)
    if (!execution) return null
    return {
      mark: execution.state === 'imported' ? 'done' : execution.state === 'lost' ? 'fail' : 'now',
      label,
      fact: execution.execution_id ? (
        <span className="eval-ui-mono">{shortId(execution.execution_id)}</span>
      ) : (
        'not dispatched yet'
      ),
    }
  }
  const measure: (Step | null)[] = [
    cycleInput(cycle).kind === 'execution'
      ? source
        ? {
            mark: 'done',
            label: 'Execution imported',
            fact: <span className="eval-ui-mono">{shortId(source.execution_id)}</span>,
          }
        : executionStep('source', 'Execution imported')
      : null,
    source
      ? {
          mark: 'done',
          label: 'First pass',
          fact: `${plural(source.findings.filter((finding) => finding.class === 'defect' || finding.class === 'improvement_opportunity').length, 'finding')} worth a change · no model called`,
        }
      : null,
    cycle.triage
      ? {
          mark: cycle.triage.outcome === 'unknown' ? 'stop' : 'done',
          label: cycle.triage.source === 'jev' ? 'Judged once' : 'Triaged',
          fact: cycle.triage.reason,
        }
      : null,
    cycle.coverage
      ? cycle.coverage.covered
        ? { mark: 'done', label: 'Coverage and plan', fact: cycle.plan ? 'Frozen before any new run' : 'Covered' }
        : { mark: 'stop', label: 'Coverage', fact: cycle.coverage.gaps[0] ?? 'No E2E test covers it' }
      : null,
    cycle.baseline
      ? {
          mark: cycle.baseline.usable ? 'done' : 'stop',
          label: 'Baseline',
          fact: cycle.baseline.usable ? (
            <>
              <span className="eval-ui-mono">{shortId(cycle.baseline.execution_id)}</span>
              {cycle.plan ? (
                <>
                  {' · '}
                  <span className="eval-ui-mono">{criterionName(cycle.plan.criterion)}</span>{' '}
                  {formatMetric(cycle.plan.criterion.metric, cycle.baseline.primary.mean)}
                </>
              ) : null}
            </>
          ) : (
            cycle.baseline.reason
          ),
        }
      : executionStep('baseline', 'Baseline'),
  ]
  const proposal = cycle.proposal?.proposal
  const candidate = cycle.candidate?.candidate
  const actorMark = (state?: string): Mark => (state === 'completed' ? 'done' : state === 'failed' ? 'fail' : 'now')
  const improve: (Step | null)[] = frozen?.candidate
    ? [
        cycle.proposal
          ? {
              mark: actorMark(proposal?.status.state),
              label: 'Proposal',
              fact: proposal?.hypothesis ?? proposal?.change ?? 'Reading the evidence',
            }
          : null,
        cycle.candidate
          ? {
              mark: actorMark(candidate?.status.state),
              label: 'Candidate',
              fact: candidate?.commit ? (
                <>
                  <span className="eval-ui-mono">{short(candidate.commit.candidate_sha)}</span> ·{' '}
                  {plural(candidate.commit.paths.length, 'file')}
                </>
              ) : (
                'Writing the change in its own worktree'
              ),
            }
          : null,
        cycle.candidate?.publish
          ? {
              mark: 'done',
              label: 'Published',
              fact: <span className="eval-ui-mono">{cycle.candidate.publish.branch}</span>,
            }
          : null,
      ]
    : reached(1)
      ? [{ mark: active ? 'off' : 'stop', label: 'Candidate work', fact: 'Off in Loop settings' }]
      : []
  // The base and the candidate alternate, one round per execution.
  const rounds = cycle.executions?.filter((entry) => entry.role === 'candidate' || entry.role === 'control') ?? []
  const planned = 2 * (cycle.plan?.sampling.runs_per_side ?? 0)
  const proved = rounds.filter((entry) => entry.state === 'imported').length
  const prove: (Step | null)[] = [
    rounds.length
      ? {
          mark: rounds.some((entry) => entry.state === 'lost')
            ? 'fail'
            : proved === planned
              ? 'done'
              : active
                ? 'now'
                : 'stop',
          label: 'Paired rounds',
          fact: `${proved} of ${planned} in · base and candidate alternate`,
        }
      : null,
    cycle.decision
      ? {
          mark: OUTCOME_MARK[cycle.decision.outcome],
          label: 'Source decision',
          fact: decisionView(cycle.decision.outcome).label,
        }
      : rounds.length
        ? { mark: 'todo', label: 'Source decision', fact: 'In code, when every round is in' }
        : null,
  ]
  const held = review(cycle)
  const pr = cycle.pull_request
  const ship: (Step | null)[] = held
    ? [
        held.kind === 'opened' && pr
          ? {
              mark: 'done',
              label: 'Pull request',
              fact: (
                <>
                  <a className="eval-ui-pr-link" href={pr.url} target="_blank" rel="noreferrer">
                    #{pr.number}
                  </a>
                  {pr.created ? ' opened' : ' found'}
                </>
              ),
            }
          : held.kind === 'waiting'
            ? { mark: 'now', label: 'Pull request', fact: held.detail }
            : { mark: 'stop', label: 'No pull request', fact: held.detail },
      ]
    : []
  return [measure, improve, prove, ship].map((list) => list.filter((step): step is Step => step !== null))
}

const LANE: Record<Mark, { word: string; tone: Tone }> = {
  done: { word: 'Done', tone: 'ok' },
  now: { word: 'Running', tone: 'accent' },
  stop: { word: 'Stopped', tone: 'warn' },
  fail: { word: 'Failed', tone: 'alert' },
  todo: { word: '', tone: 'neutral' },
  off: { word: 'Off', tone: 'neutral' },
}

/** The four phases with their steps; a finished cycle shows only each phase's last step. */
function Path({ cycle, current, narrow }: { cycle: CycleRecord; current: LoopConfig | null; narrow: boolean }) {
  const all = steps(cycle, current)
  const lanes = marks(cycle)
  const at = phaseOf(cycle)
  const active = isActive(cycle.status)
  return (
    <section aria-label="Where the cycle is" className="eval-ui-section">
      <h3 className="eval-ui-h">Where the cycle is</h3>
      <ol className="eval-ui-lanes" data-narrow={narrow ? 'true' : undefined}>
        {PHASES.map((phase, index) => {
          const lane = LANE[lanes[index]]
          const list = active ? all[index] : all[index].slice(-1)
          // The step that says where the phase stands: the live or stopped one, else the last.
          const current_ =
            list.find((step) => step.mark === 'now' || step.mark === 'stop' || step.mark === 'fail') ?? list.at(-1)
          const body = list.length ? (
            list.map((step) => (
              <div key={step.label} className="eval-ui-step">
                <MarkIcon mark={step.mark} />
                <p
                  className={
                    step.mark === 'todo' || step.mark === 'off' ? 'eval-ui-strong eval-ui-faint' : 'eval-ui-strong'
                  }
                >
                  {step.label}
                </p>
                {step.fact ? <p className="eval-ui-step-fact">{step.fact}</p> : null}
              </div>
            ))
          ) : (
            <div className="eval-ui-step">
              <MarkIcon mark="todo" />
              <p className="eval-ui-faint">Not reached</p>
            </div>
          )
          return (
            <li key={phase} className="eval-ui-lane" aria-current={index === at ? 'step' : undefined}>
              {narrow ? (
                <details open={index === at}>
                  <summary className="eval-ui-lane-sum">
                    <MarkIcon mark={lanes[index]} />
                    <span className="eval-ui-lane-text">
                      <span className="eval-ui-strong">{phase}</span>
                      <span className="eval-ui-faint eval-ui-clip">
                        {current_ ? (
                          <>
                            {current_.label}
                            {current_.fact ? <> · {current_.fact}</> : null}
                          </>
                        ) : (
                          'Not reached'
                        )}
                      </span>
                    </span>
                    {lane.word ? <span className="eval-ui-faint">{lane.word}</span> : null}
                    <Caret />
                  </summary>
                  {body}
                </details>
              ) : (
                <>
                  <div className="eval-ui-lane-head">
                    <Eyebrow as="p">
                      {index + 1} · {phase}
                    </Eyebrow>
                    {lane.word ? <StatusChip tone={lane.tone} label={lane.word} /> : null}
                  </div>
                  {body}
                </>
              )}
            </li>
          )
        })}
      </ol>
    </section>
  )
}

// ── the decision ────────────────────────────────────────────────────────

/** The frozen rule, before the decision exists. */
function Rule({ cycle }: { cycle: CycleRecord }) {
  const plan = cycle.plan
  if (!plan) return null
  const base = cycle.baseline
  return (
    <Panel>
      <PanelHeader>
        <span>
          How the decision is made <span className="eval-ui-faint eval-ui-light">· frozen when the cycle started</span>
        </span>
      </PanelHeader>
      <PanelBody className="eval-ui-stack">
        <div className="eval-ui-rule">
          <Eyebrow as="p">Must improve</Eyebrow>
          <Eyebrow as="p">Baseline → candidate</Eyebrow>
          <p>
            <Name criterion={plan.criterion} /> {plan.criterion.direction === 'increase' ? 'rises' : 'falls'} by at
            least <span className="eval-ui-num">{formatEffect(plan.criterion.metric, plan.criterion.min_effect)}</span>
          </p>
          <p className="eval-ui-num">
            {base ? formatMetric(plan.criterion.metric, base.primary.mean) : '—'} →{' '}
            <span className="eval-ui-ghost">
              {cycle.decision ? '' : isActive(cycle.status) ? 'running' : 'not run'}
            </span>
          </p>
        </div>
        <p className="eval-ui-faint">
          The base runs again beside the candidate: {plural(plan.sampling.runs_per_side, 'round')} each, alternating one
          round at a time, compared round by round. The change is accepted only if every round improves or the rounds
          agree at p ≤ 0.05, and nothing below drops. Model, provider, engine, stack, cases and policy must match on
          both sides.
        </p>
        {plan.preserved?.length ? (
          <div className="eval-ui-stack-sm">
            <Eyebrow as="p">Must not drop</Eyebrow>
            <div className="eval-ui-chips">
              {plan.preserved.map((criterion, index) => (
                <Chip key={`${criterion.scenario_id}:${criterionName(criterion)}`} className="eval-ui-chip">
                  {keepName(criterion, plan.criterion.scenario_id)}
                  {base?.preserved[index]?.mean !== undefined ? (
                    <span className="eval-ui-num eval-ui-faint">
                      {formatMetric(criterion.metric, base.preserved[index].mean)}
                    </span>
                  ) : null}
                </Chip>
              ))}
            </div>
          </div>
        ) : null}
      </PanelBody>
    </Panel>
  )
}

/** A criterion's name: an id in mono, a metric in words. */
const Name = ({ criterion }: { criterion: Criterion }) =>
  criterion.criterion_id ? <span className="eval-ui-mono">{criterion.criterion_id}</span> : criterionName(criterion)

/** A preserved criterion, named with its scenario when it is not the primary one. */
const keepName = (criterion: Criterion, primary: string) => (
  <>
    <Name criterion={criterion} />
    {criterion.scenario_id === primary ? null : (
      <>
        {' on '}
        <span className="eval-ui-mono">{criterion.scenario_id}</span>
      </>
    )}
  </>
)

const KEPT: Record<Evidence['computed_outcome'], { label: string; tone: Tone }> = {
  validated_improvement: { label: 'Improved', tone: 'ok' },
  no_improvement: { label: 'Held', tone: 'ok' },
  regression: { label: 'Lost', tone: 'alert' },
  inconclusive: { label: 'Unknown', tone: 'warn' },
}

/** A decision in full; `brief` only says what it decided, for one that repeats the source decision's rule. */
function Decision({
  title,
  cycle,
  decision,
  brief,
}: {
  title: string
  cycle: CycleRecord
  decision: CycleDecision
  brief?: boolean
}) {
  const view = decisionView(decision.outcome)
  const criterion = cycle.plan?.criterion
  const primary = decision.primary
  const matched = decision.comparability.checks.filter((check) => check.matches).length
  return (
    <Panel>
      <PanelHeader className="eval-ui-panel-head">
        <span>{title}</span>
        <StatusChip tone={view.tone} label={view.label} />
      </PanelHeader>
      <PanelBody className="eval-ui-stack">
        {criterion ? (
          <p className="eval-ui-lead">
            <Name criterion={criterion} /> went from{' '}
            <span className="eval-ui-num">{formatMetric(criterion.metric, primary.baseline.mean)}</span> to{' '}
            <span className="eval-ui-num">{formatMetric(criterion.metric, primary.candidate.mean)}</span> (
            <span className="eval-ui-num">{deltaLine(criterion.metric, primary)}</span>). {decision.reason}
          </p>
        ) : (
          <p className="eval-ui-lead">{decision.reason}</p>
        )}
        {criterion && !brief ? <Pairs criterion={criterion} evidence={primary} /> : null}
        {decision.preserved.length && !brief ? (
          <div className="eval-ui-stack-sm">
            <Eyebrow as="p">
              {decision.preserved.every((evidence) => evidence.computed_outcome !== 'regression')
                ? 'Nothing it had to keep was lost'
                : 'What it had to keep'}
            </Eyebrow>
            <TableViewport>
              <Table density="compact" inset>
                <TableHeader>
                  <TableRow>
                    <TableHead>Criterion</TableHead>
                    <TableHead className="eval-ui-right">Baseline</TableHead>
                    <TableHead className="eval-ui-right">Candidate</TableHead>
                    <TableHead>Result</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {decision.preserved.map((evidence, index) => {
                    const kept = cycle.plan?.preserved?.[index]
                    const result = KEPT[evidence.computed_outcome]
                    return (
                      <TableRow key={`${evidence.scenario_id}:${index}`}>
                        <TableCell>
                          {kept ? keepName(kept, criterion?.scenario_id ?? '') : evidence.scenario_id}
                        </TableCell>
                        <TableCell className="eval-ui-right eval-ui-num">
                          {kept ? formatMetric(kept.metric, evidence.baseline.mean) : '—'}
                        </TableCell>
                        <TableCell className="eval-ui-right eval-ui-num">
                          {kept ? formatMetric(kept.metric, evidence.candidate.mean) : '—'}
                        </TableCell>
                        <TableCell>
                          <span title={evidence.reason}>
                            <StatusChip tone={result.tone} label={result.label} />
                          </span>
                        </TableCell>
                      </TableRow>
                    )
                  })}
                </TableBody>
              </Table>
            </TableViewport>
          </div>
        ) : null}
        <details className="eval-ui-details">
          <summary>
            <span>
              Comparability ·{' '}
              <span className="eval-ui-num">{`${matched} of ${decision.comparability.checks.length}`}</span> checks
              matched
            </span>
            <Caret />
          </summary>
          <ul className="eval-ui-checks">
            {decision.comparability.checks.map((check) => (
              <li key={check.field}>
                <MarkIcon mark={check.matches ? 'done' : 'fail'} />
                <span className="eval-ui-mono">{check.field}</span>
                <span className="eval-ui-mono eval-ui-faint eval-ui-clip">
                  {check.baseline ?? 'unknown'} → {check.candidate ?? 'unknown'}
                </span>
              </li>
            ))}
          </ul>
        </details>
      </PanelBody>
    </Panel>
  )
}

const RATES = new Set(['pass_rate', 'criterion_score'])

/** Each baseline run beside its paired candidate run, on one scale. */
function Pairs({ criterion, evidence }: { criterion: Criterion; evidence: Evidence }) {
  const pairs = evidence.baseline.runs.map((run, index) => {
    const other = evidence.candidate.runs[index]
    return [runValue(run, criterion), other ? runValue(other, criterion) : undefined] as const
  })
  if (pairs.length === 0) return null
  const values = pairs.flat().filter((value): value is number => value !== undefined)
  const max = RATES.has(criterion.metric) ? 1 : Math.max(...values, 0) || 1
  const at = (value: number) => `${(value / max) * 100}%`
  const rate = RATES.has(criterion.metric)
  const change = (before?: number, after?: number) =>
    before === undefined || after === undefined
      ? 'unmeasured'
      : rate
        ? `${after >= before ? '+' : '−'}${Math.abs((after - before) * 100).toFixed(0)} pp`
        : `${after >= before ? '+' : '−'}${formatMetric(criterion.metric, Math.abs(after - before))}`
  return (
    <figure className="eval-ui-pairs" aria-label={`Paired runs of ${criterionName(criterion)}`}>
      <div className="eval-ui-pairs-grid">
        <span />
        <span className="eval-ui-pairs-axis" aria-hidden>
          <span>{formatMetric(criterion.metric, 0)}</span>
          <span>{formatMetric(criterion.metric, max)}</span>
        </span>
        <Eyebrow as="span" className="eval-ui-right">
          Change
        </Eyebrow>
        {pairs.map(([before, after], index) => (
          <PairRow
            key={evidence.baseline.runs[index].run_id}
            index={index}
            before={before}
            after={after}
            at={at}
            change={change(before, after)}
          />
        ))}
      </div>
      <figcaption className="eval-ui-legend">
        <span>
          <span className="eval-ui-pin" data-side="baseline" /> Baseline
        </span>
        <span>
          <span className="eval-ui-pin" data-side="candidate" /> Candidate
        </span>
      </figcaption>
    </figure>
  )
}

function PairRow({
  index,
  before,
  after,
  at,
  change,
}: {
  index: number
  before?: number
  after?: number
  at: (value: number) => string
  change: string
}) {
  const low = Math.min(before ?? 0, after ?? 0)
  const high = Math.max(before ?? 0, after ?? 0)
  return (
    <>
      <span className="eval-ui-num eval-ui-faint">run {index + 1}</span>
      <span className="eval-ui-track">
        {before !== undefined && after !== undefined ? (
          <span className="eval-ui-span" style={{ left: at(low), width: `calc(${at(high)} - ${at(low)})` }} />
        ) : null}
        {before !== undefined ? (
          <span className="eval-ui-pin" data-side="baseline" style={{ left: at(before) }} />
        ) : null}
        {after !== undefined ? (
          <span className="eval-ui-pin" data-side="candidate" style={{ left: at(after) }} />
        ) : null}
      </span>
      <span className="eval-ui-num eval-ui-right">{change}</span>
    </>
  )
}

// ── findings, unknowns, activity ────────────────────────────────────────

function Findings({ cycle, collapsed }: { cycle: CycleRecord; collapsed: boolean }) {
  const findings = (cycle.analyses ?? []).flatMap((analysis) =>
    analysis.findings
      .filter((finding) => finding.class !== 'expected_behavior')
      .map((finding) => ({ ...finding, role: analysis.role })),
  )
  if (findings.length === 0) return null
  const table = (
    <TableViewport>
      <Table density="compact" inset>
        <TableHeader>
          <TableRow>
            <TableHead>Kind</TableHead>
            <TableHead>Criterion</TableHead>
            <TableHead>What happened</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {findings.map((finding) => {
            const view = findingView(finding.class)
            // The first pass writes `run …: criterion <id> awarded <n> of <m>: <why>`.
            const award = finding.observation.match(/criterion (\S+) (awarded .*)$/)
            return (
              <TableRow key={`${finding.role}:${finding.fingerprint}`}>
                <TableCell>
                  <StatusChip tone={view.tone} label={view.label} />
                </TableCell>
                <TableCell className="eval-ui-mono eval-ui-nowrap">{award?.[1] ?? finding.signal}</TableCell>
                <TableCell>{award ? sentence(award[2]) : finding.observation}</TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </TableViewport>
  )
  // An ended cycle says what was lost above; the run-by-run rows stay one click away.
  if (collapsed) {
    return (
      <details className="eval-ui-details">
        <summary>
          <span>
            Run by run · <span className="eval-ui-num">{plural(findings.length, 'finding')}</span>
          </span>
          <Caret />
        </summary>
        {table}
      </details>
    )
  }
  return (
    <Panel>
      <PanelHeader className="eval-ui-panel-head">
        <span>What the first pass found</span>
        <Badge className="eval-ui-num">{findings.length}</Badge>
      </PanelHeader>
      {table}
    </Panel>
  )
}

function Unknowns({ host, cycle, found }: { host: Host; cycle: CycleRecord; found: ReturnType<typeof gaps> }) {
  const group = (cycle.analyses ?? [])
    .flatMap((analysis) => (analysis.sentinel?.results ?? []).map(sentinelResult))
    .find((result) => result?.group_id)?.group_id
  return (
    <details className="eval-ui-details" aria-label="What the loop could not tell">
      <summary>
        <span>
          What the loop could not tell · <span className="eval-ui-num">{found.length}</span>
        </span>
        <Caret />
      </summary>
      <ul className="eval-ui-events">
        {found.map((gap, index) => (
          <li key={`${gap.kind}-${index}`}>
            <Info size={16} className="eval-ui-ghost" aria-hidden />
            <div>
              <p>{gap.reason}</p>
              <p className="eval-ui-faint">{gap.subject}</p>
            </div>
          </li>
        ))}
      </ul>
      {group && host.panels ? (
        <Button
          variant="ghost"
          size="sm"
          className="eval-ui-start"
          onClick={() => host.panels?.open({ pageId: SENTINEL_PAGE, context: { group_id: group } })}
        >
          Open in Sentinel
        </Button>
      ) : null}
    </details>
  )
}

function Activity({ cycle, collapsed }: { cycle: CycleRecord; collapsed: boolean }) {
  const [all, setAll] = useState(false)
  const events = [...cycle.history].reverse()
  const shown = all ? events : events.slice(0, ACTIVITY_SHOWN)
  const list = (
    <>
      <ol className="eval-ui-events">
        {shown.map((event, index) => (
          <li key={`${event.at}-${index}`}>
            <MarkIcon
              mark={index === 0 && isActive(cycle.status) ? 'now' : event.status === 'failed' ? 'fail' : 'done'}
            />
            <p className={index === 0 ? undefined : 'eval-ui-faint'}>{sentence(event.detail)}</p>
            <time
              className="eval-ui-num eval-ui-ghost"
              dateTime={new Date(event.at).toISOString()}
              title={new Date(event.at).toLocaleString()}
            >
              {clock(event.at)}
            </time>
          </li>
        ))}
      </ol>
      {events.length > ACTIVITY_SHOWN ? (
        <Button variant="ghost" size="sm" className="eval-ui-start" onClick={() => setAll(!all)}>
          {all ? 'Show the latest' : `Show all ${events.length} events`}
        </Button>
      ) : null}
    </>
  )
  // An ended cycle's step log is for digging; the result above says what happened.
  return collapsed ? (
    <details className="eval-ui-details" aria-label="Activity">
      <summary>
        <span>
          Activity · <span className="eval-ui-num">{plural(events.length, 'event')}</span>
        </span>
        <Caret />
      </summary>
      {list}
    </details>
  ) : (
    <section aria-label="Activity" className="eval-ui-section">
      <h3 className="eval-ui-h">Activity</h3>
      {list}
    </section>
  )
}

/** Everything the record holds (frozen tests, digests, Sentinel and judge answers), read only when opened. */
function FullRecord({ cycle }: { cycle: CycleRecord }) {
  const [open, setOpen] = useState(false)
  return (
    <details className="eval-ui-details" onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>
        <span>Full record</span>
        <Caret />
      </summary>
      {open ? <JsonHighlight code={JSON.stringify(cycle, null, 2)} wrap /> : null}
    </details>
  )
}

// ── the aside ───────────────────────────────────────────────────────────

function Meter({ label, used, limit, text }: { label: string; used: number; limit: number; text: string }) {
  const share = limit > 0 ? Math.min(100, Math.round((used / limit) * 100)) : 0
  return (
    <div className="eval-ui-stack-sm">
      <div className="eval-ui-meter-head">
        <Eyebrow as="span">{label}</Eyebrow>
        <span className="eval-ui-num">{text}</span>
      </div>
      {/* The text above says it; the bar only shows it. */}
      <div className="eval-ui-meter" aria-hidden data-tone={share >= 80 ? 'warn' : undefined}>
        <span style={{ width: `${share}%` }} />
      </div>
    </div>
  )
}

function Budget({ usage }: { usage: CycleUsage | null }) {
  return (
    <Panel>
      <PanelHeader>Budget</PanelHeader>
      <PanelBody className="eval-ui-stack">
        {usage ? (
          <>
            <div className="eval-ui-meters">
              <Meter
                label="Spend"
                used={usage.spend_usd}
                limit={usage.budget.max_spend_usd}
                text={`${usd(usage.spend_usd)} of ${usd(usage.budget.max_spend_usd)}`}
              />
              <Meter
                label="Time"
                used={usage.elapsed_ms}
                limit={usage.budget.max_duration_ms}
                text={`${span(usage.elapsed_ms)} of ${span(usage.budget.max_duration_ms)}`}
              />
              <Meter
                label="Calls"
                used={usage.calls}
                limit={usage.budget.max_calls}
                text={`${usage.calls} of ${usage.budget.max_calls}`}
              />
              <Meter
                label="Candidates"
                used={usage.candidates}
                limit={usage.budget.max_candidates}
                text={`${usage.candidates} of ${usage.budget.max_candidates}`}
              />
            </div>
            <p className="eval-ui-faint">
              Work is reserved before it starts; a cost nobody reported stays charged at its estimate
              {usage.unknown_cost ? ` (${usage.unknown_cost} item(s) here)` : ''}.
            </p>
          </>
        ) : (
          <p className="eval-ui-faint">No work reserved yet: nothing spent.</p>
        )}
      </PanelBody>
    </Panel>
  )
}

function Details({ cycle }: { cycle: CycleRecord }) {
  const input = cycleInput(cycle)
  const plan = cycle.plan
  const frozen = cycle.config
  const source = cycle.analyses?.find((analysis) => analysis.role === 'source')
  const subject = plan?.parameters ?? frozen?.config.subject
  const model = subject?.model ?? source?.identity.model
  const facts: [string, ReactNode][] = [
    [
      input.kind === 'objective' ? 'Objective' : 'Source',
      input.kind === 'objective' ? (
        input.value
      ) : (
        <span className="eval-ui-mono eval-ui-clip" title={input.value}>
          {shortId(input.value)}
        </span>
      ),
    ],
    ['Model', model ? <span className="eval-ui-mono eval-ui-clip">{model}</span> : 'unknown'],
    [
      'Runs on',
      `${plan?.parameters.where ?? frozen?.config.where ?? 'not configured'}${
        frozen?.config.validation ? ` · candidate on ${frozen.config.validation.where}` : ''
      }`,
    ],
    ['Started', <span key="s" className="eval-ui-num">{`${clock(cycle.created_at)} · ${ago(cycle.created_at)}`}</span>],
    [
      'Policy',
      plan || frozen ? (
        <span className="eval-ui-mono">{(plan?.policy_revision ?? frozen?.revision ?? '').slice(0, 8)}</span>
      ) : (
        'unconfigured'
      ),
    ],
    [
      'Cycle',
      <span key="c" className="eval-ui-mono eval-ui-clip">
        {cycle.cycle_id}
      </span>,
    ],
  ]
  return (
    <Panel>
      <PanelHeader>Details</PanelHeader>
      <dl className="eval-ui-kv">
        {facts.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </Panel>
  )
}
