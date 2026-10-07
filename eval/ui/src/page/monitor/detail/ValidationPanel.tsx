// "E2E runs" of one suggestion: the latest baseline/candidate link as the E2E
// service reported it, the signal the monitor counted in each run, the outcome
// the code proposes from the registered criterion, the measures of each
// scenario with the difference between the sides, and the person's verdict.
// Differences are computed in code and never colored; unknown stays unknown.
import {
  Badge,
  Button,
  CardHighlight,
  Chip,
  StatusDot,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  uiClasses,
} from '@iii-dev/console-ui'
import { ChevronDown, ChevronUp, ExternalLink, Lock, TriangleAlert, Unlink } from 'lucide-react'
import { useId, useState } from 'react'
import type {
  Criterion,
  E2eExecution,
  Evidence,
  EvidenceRun,
  SuggestionReview,
  ValidationLink,
  ValidationPlan,
} from '../../../types'
import { Proposal, VerdictTile } from './ReviewVerdict'
import { criterionSummary, patternParts, registeredLate } from './review-model'
import { CheckList, HarnessLine, Mismatches } from './validation-parts'
import {
  type DiffRow,
  formatClock,
  hasMeasures,
  latestLink,
  type MeasureCell,
  mismatches,
  reportProblem,
  reportsAvailable,
  type ScenarioTable,
  scenarioIds,
  scenarioTables,
  signalRow,
} from './validation-view'

const PROPOSAL_NOTE = 'A proposal computed from the criterion and the numbers. It is not your verdict.'
const MEASURES_NOTE =
  "Δ is candidate minus baseline, computed in code. Unreported values stay unreported: a side with no cost shows “not reported”, never $0, and the Δ is left blank. The monitor doesn't color these numbers."

function Cell({ cell }: { cell: MeasureCell }) {
  if (cell.value === undefined && !cell.sub && !cell.warn) return <span className="eval-ui-val-quiet">—</span>
  return (
    <span className="eval-ui-val-cell">
      {cell.value !== undefined ? <span className="eval-ui-val-value">{cell.value}</span> : null}
      {cell.sub ? <span className="eval-ui-val-sub">{cell.sub}</span> : null}
      {cell.warn ? <span className="eval-ui-val-warn">{cell.warn}</span> : null}
    </span>
  )
}

function Delta({ row }: { row: DiffRow }) {
  if (!row.delta) return <span className="eval-ui-val-quiet">—</span>
  return (
    <span className="eval-ui-val-cell">
      <span className="eval-ui-val-value">{row.delta.abs}</span>
      {row.delta.pct ? <span className="eval-ui-val-sub">{row.delta.pct}</span> : null}
    </span>
  )
}

function RowLabel({ row }: { row: DiffRow }) {
  return (
    <span className="eval-ui-val-measure">
      <span className="eval-ui-val-measure-name">
        {row.label}
        {row.qualifier ? <span className="eval-ui-val-sub"> {row.qualifier}</span> : null}
      </span>
      {row.primary ? <Chip className="eval-ui-val-primary">Primary</Chip> : null}
    </span>
  )
}

/** `Baseline`, the Harness it ran, and how many runs: the head of a column. */
function SideHead({ label, run, runs }: { label: string; run: E2eExecution; runs?: number }) {
  return (
    <span className="eval-ui-val-runhead">
      <span className="eval-ui-val-ink">{label}</span>
      <span className="eval-ui-val-sub">
        {run.harness_version ? `harness ${run.harness_version}` : 'harness not reported'}
      </span>
      {runs !== undefined ? <span className="eval-ui-val-sub">{runs} runs</span> : null}
    </span>
  )
}

function ScenarioRows({ table }: { table: ScenarioTable }) {
  return (
    <>
      <TableRow>
        <TableCell colSpan={4} className="eval-ui-val-scenario">
          <span className="eval-ui-val-mono eval-ui-val-ink">{table.scenarioId}</span>
          <span className="eval-ui-val-quiet eval-ui-val-small">
            {' '}
            {table.target ? 'target' : 'other'} · {table.runs}
          </span>
          {table.caution ? <span className="eval-ui-val-warn eval-ui-val-caution">{table.caution}</span> : null}
        </TableCell>
      </TableRow>
      {table.rows.map((row) => (
        <TableRow key={row.key}>
          <TableHead scope="row" className="eval-ui-val-rowhead">
            <RowLabel row={row} />
          </TableHead>
          <TableCell className="eval-ui-val-num">
            <Cell cell={row.baseline} />
          </TableCell>
          <TableCell className="eval-ui-val-num">
            <Cell cell={row.candidate} />
          </TableCell>
          <TableCell className="eval-ui-val-num">
            <Delta row={row} />
          </TableCell>
        </TableRow>
      ))}
    </>
  )
}

/** The phone's version of a scenario: each measure a block, the three values under it. */
function ScenarioBlock({ table }: { table: ScenarioTable }) {
  return (
    <div className="eval-ui-val-block-scenario">
      <p className="eval-ui-val-scenario-title">
        <span className="eval-ui-val-mono eval-ui-val-ink">{table.scenarioId}</span>
        <span className="eval-ui-val-quiet eval-ui-val-small">
          {' '}
          {table.target ? 'target' : 'other'} · {table.runs}
        </span>
        {table.caution ? <span className="eval-ui-val-warn eval-ui-val-caution">{table.caution}</span> : null}
      </p>
      {table.rows.map((row) => (
        <div key={row.key} className="eval-ui-val-stat">
          <RowLabel row={row} />
          <div className="eval-ui-val-stat-grid">
            <span className="eval-ui-val-stat-name">Baseline</span>
            <span className="eval-ui-val-stat-name">Candidate</span>
            <span className="eval-ui-val-stat-name">Δ</span>
            <Cell cell={row.baseline} />
            <Cell cell={row.candidate} />
            <Delta row={row} />
          </div>
        </div>
      ))}
    </div>
  )
}

function Measures({ link, tables, labelledBy }: { link: ValidationLink; tables: ScenarioTable[]; labelledBy: string }) {
  const [showOthers, setShowOthers] = useState(false)
  const [target, ...others] = tables
  const toggle =
    others.length > 0 ? (
      <button
        type="button"
        className="eval-ui-val-others"
        aria-expanded={showOthers}
        onClick={() => setShowOthers((now) => !now)}
      >
        <span>
          {others.length} other {others.length === 1 ? 'scenario' : 'scenarios'} in these executions · not averaged
        </span>
        <span className="eval-ui-val-others-hint">
          {showOthers ? 'Hide' : 'Show'}
          {showOthers ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
        </span>
      </button>
    ) : null
  const shown = showOthers ? others : []
  return (
    <>
      <div className="eval-ui-val-wide">
        <TableViewport>
          <TableFrame>
            <Table density="compact" className="eval-ui-val-table" aria-labelledby={labelledBy}>
              <TableHeader>
                <TableRow>
                  <TableHead scope="col" className="eval-ui-val-col-measure">
                    Measure
                  </TableHead>
                  <TableHead scope="col" className="eval-ui-val-num eval-ui-val-col-side">
                    <SideHead label="Baseline" run={link.baseline} />
                  </TableHead>
                  <TableHead scope="col" className="eval-ui-val-num eval-ui-val-col-side">
                    <SideHead label="Candidate" run={link.candidate} />
                  </TableHead>
                  <TableHead scope="col" className="eval-ui-val-num eval-ui-val-col-delta">
                    Δ
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <ScenarioRows table={target} />
                {shown.map((table) => (
                  <ScenarioRows key={table.scenarioId} table={table} />
                ))}
              </TableBody>
            </Table>
          </TableFrame>
        </TableViewport>
      </div>
      <div className="eval-ui-val-narrow">
        <ScenarioBlock table={target} />
        {shown.map((table) => (
          <ScenarioBlock key={table.scenarioId} table={table} />
        ))}
      </div>
      {toggle}
    </>
  )
}

function ReportNotice({ problem }: { problem: { ids: string[]; errors: string[] } }) {
  const ids = problem.ids.join(' and ')
  return (
    <StatusPanel
      variant="warn"
      role="status"
      icon={<Unlink className={uiClasses.icon} aria-hidden />}
      headline="Runs linked, report unavailable"
      detail={
        <>
          {"The E2E service didn't return the report for "}
          <span className="eval-ui-val-mono eval-ui-val-ink">{ids}</span>. The IDs are saved; the numbers below are
          missing what that report holds.
          {problem.errors.map((error) => (
            <span key={error} className="eval-ui-val-mono eval-ui-val-error">
              {error}
            </span>
          ))}
        </>
      }
    />
  )
}

/** One chip per run of a side: the count, or why the run is not counted. */
function RunChips({ label, runs, pattern }: { label: string; runs: EvidenceRun[]; pattern: string }) {
  return (
    <div className="eval-ui-val-runs">
      <span className="eval-ui-val-quiet eval-ui-val-small">{label}</span>
      <div className="eval-ui-val-chips">
        {runs.map((run, at) => {
          const count = run.signals?.[pattern]
          const why = run.completed
            ? undefined
            : (run.technical !== 'valid' && run.technical) || run.completion || 'incomplete'
          return (
            <Chip
              key={run.run_id}
              className="eval-ui-val-mono"
              data-uncounted={run.completed && count !== undefined ? undefined : true}
              title={why ? `Run ${at + 1}: ${why.replaceAll('_', ' ')}, not counted` : `Run ${at + 1}`}
            >
              {why ? '×' : (count ?? '—')}
            </Chip>
          )
        })}
      </div>
    </div>
  )
}

/** The signal the criterion counts, as the code counted it in each run's transcript. */
function SignalEvidence({ criterion, evidence }: { criterion: Criterion; evidence: Evidence }) {
  const pattern = criterion.pattern ?? ''
  const mean = (value: number | undefined) => (value === undefined ? 'no value' : value.toFixed(1))
  return (
    <div className="eval-ui-val-evidence">
      <div className="eval-ui-val-row">
        <span className="eval-ui-val-label">Signal counted in the runs</span>
        <span className="eval-ui-val-quiet eval-ui-val-small">by the monitor, from each run’s transcript</span>
      </div>
      <p className="eval-ui-val-evidence-line">
        <span className="eval-ui-val-mono eval-ui-val-ink">{patternParts(pattern).rule}</span> per run:{' '}
        <strong className="eval-ui-val-evidence-figure">
          {mean(evidence.baseline.mean)} → {mean(evidence.candidate.mean)}
        </strong>{' '}
        <span className="eval-ui-val-quiet eval-ui-val-small">
          (n={evidence.baseline.n}/{evidence.candidate.n})
        </span>
      </p>
      <div className="eval-ui-val-evidence-runs">
        <RunChips label="Baseline runs" runs={evidence.baseline.runs} pattern={pattern} />
        <RunChips label="Candidate runs" runs={evidence.candidate.runs} pattern={pattern} />
      </div>
      <p className="eval-ui-val-note">
        The same rule that raised the signal in the observed session, run over each run’s root session. Sub-sessions of
        Docker runs are not read.
      </p>
    </div>
  )
}

export function ValidationPanel({
  links,
  review,
  plan,
  narrow,
  onVerdict,
  onOpenE2e,
}: {
  links: ValidationLink[]
  review: SuggestionReview
  plan: ValidationPlan
  narrow: boolean
  /** Opens the verdict dialog (record, or change). */
  onVerdict: () => void
  onOpenE2e?: () => void
}) {
  const titleId = useId()
  const found = latestLink(links)
  if (!found) return null
  const { latest: link, earlier } = found
  const { comparable, checks } = link.comparability
  const different = mismatches(checks)
  const matching = checks.filter((check) => check.matches)
  const problem = reportProblem(link)
  const scenarios = [...new Set([...scenarioIds(link.baseline), ...scenarioIds(link.candidate)])]
  const { criterion, evidence, run } = review
  const late = registeredLate(review, links)
  const tables = scenarioTables(link, {
    target: criterion?.scenario_id ?? evidence?.scenario_id ?? review.scenario_id,
    primary: criterion?.metric,
    signal: signalRow(criterion, evidence),
  })
  // The pair was attached by the validation this worker started, not by hand.
  const started = run !== undefined && run.state === 'attached' && link.attached_at >= run.started_at

  return (
    <div className="eval-ui-val" data-narrow={narrow || undefined}>
      <CardHighlight className="eval-ui-val-panel" role="region" aria-labelledby={titleId}>
        <div className="eval-ui-val-head">
          <h4 id={titleId} className="eval-ui-val-heading">
            E2E runs
          </h4>
          {scenarios.map((scenario) => (
            <Chip key={scenario} className="eval-ui-val-mono">
              {scenario}
            </Chip>
          ))}
          {started ? (
            <Badge className="eval-ui-ad-pill">
              <span className="eval-ui-val-ring" aria-hidden />
              Started from this suggestion
            </Badge>
          ) : null}
          <span className="eval-ui-val-quiet eval-ui-val-earlier">
            attached {formatClock(link.attached_at)}
            {earlier > 0 ? ` · ${earlier} earlier ${earlier === 1 ? 'link' : 'links'}` : ''}
          </span>
        </div>

        {criterion ? (
          <p className="eval-ui-val-line eval-ui-val-criterion" data-tone={late === undefined ? 'ok' : 'warn'}>
            {late === undefined ? (
              <Lock className={uiClasses.icon} aria-hidden />
            ) : (
              <TriangleAlert className={uiClasses.icon} aria-hidden />
            )}
            <span>
              Criterion registered {formatClock(criterion.registered_at)} by {criterion.registered_by}
              {late === undefined ? ', before any run' : `, after the first run (${formatClock(late)})`} ·{' '}
              {criterionSummary(criterion)}
            </span>
          </p>
        ) : (
          <p className="eval-ui-val-note">
            No criterion was registered before these runs, so the code proposes nothing and “Validated improvement”
            isn't available.
          </p>
        )}

        {problem ? <ReportNotice problem={problem} /> : null}

        <div className="eval-ui-val-section">
          <div className="eval-ui-val-row">
            <span className="eval-ui-val-label">Comparability</span>
            <Badge variant={comparable ? 'ok' : 'alert'} className="eval-ui-ad-pill">
              <StatusDot tone={comparable ? 'ok' : 'alert'} />
              {comparable ? 'Comparable' : 'Not comparable'}
            </Badge>
            <span className="eval-ui-val-quiet eval-ui-val-small">
              {comparable
                ? `${matching.length} of ${checks.length} identity checks match`
                : `${different.length} of ${checks.length} identity checks differ`}
            </span>
          </div>
          {comparable ? null : (
            <StatusPanel
              variant="alert"
              role="alert"
              icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
              headline="These runs are not comparable"
              detail={
                <>
                  Baseline and candidate must differ only in the Harness version. A difference in the measures below may
                  come from these values instead.
                  <Mismatches checks={different} />
                </>
              }
            />
          )}
          {matching.length > 0 ? <CheckList checks={matching} values /> : null}
          <div className="eval-ui-val-quiet eval-ui-val-small">
            <HarnessLine baseline={link.baseline} candidate={link.candidate} />
          </div>
        </div>

        {criterion?.metric === 'signal_per_run' && evidence ? (
          <SignalEvidence criterion={criterion} evidence={evidence} />
        ) : null}
        {evidence ? <Proposal evidence={evidence} note={PROPOSAL_NOTE} /> : null}

        {hasMeasures(link) ? (
          <>
            <Measures link={link} tables={tables} labelledBy={titleId} />
            <p className="eval-ui-val-note">
              {MEASURES_NOTE}
              {comparable ? '' : ' Shown for reference only.'}
            </p>
          </>
        ) : null}

        <div className="eval-ui-val-chips">
          <Chip className="eval-ui-val-mono">{link.baseline.execution_id} · baseline</Chip>
          <Chip className="eval-ui-val-mono">{link.candidate.execution_id} · candidate</Chip>
          <span className="eval-ui-val-mono eval-ui-val-quiet">
            assets {reportsAvailable(link) ? 'available' : 'unavailable'}
          </span>
          {onOpenE2e ? (
            <Button variant="ghost" size={narrow ? 'lg' : 'sm'} className="eval-ui-val-open" onClick={onOpenE2e}>
              Open in E2E
              <ExternalLink size={16} aria-hidden="true" />
            </Button>
          ) : null}
        </div>

        {review.verdict ? (
          <VerdictTile review={review} plan={plan} narrow={narrow} onChange={onVerdict} />
        ) : (
          <div className="eval-ui-rv-cta">
            <div className="eval-ui-rv-cta-copy">
              <span className="eval-ui-val-label">Your verdict</span>
              <span className="eval-ui-val-quiet eval-ui-val-small">
                Not recorded yet. Check the runs, then decide.
              </span>
            </div>
            <Button variant="primary" size={narrow ? 'lg' : 'sm'} onClick={onVerdict}>
              Record verdict
            </Button>
          </div>
        )}
      </CardHighlight>
    </div>
  )
}
