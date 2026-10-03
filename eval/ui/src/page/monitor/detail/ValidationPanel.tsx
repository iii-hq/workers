// "E2E runs" of one suggestion: the latest baseline/candidate link as the E2E
// service reported it. Each execution is shown on its own; there is no
// verdict and no difference column because the service returns neither.
import {
  Badge,
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
import { Info, TriangleAlert, Unlink } from 'lucide-react'
import { useId } from 'react'
import type { E2eExecution, ValidationLink } from '../../../types'
import { CheckList, HarnessLine, Mismatches } from './validation-parts'
import {
  formatClock,
  hasMeasures,
  latestLink,
  type MeasureCell,
  measureRows,
  mismatches,
  reportProblem,
  reportsAvailable,
  scenarioIds,
} from './validation-view'

// Copy kept with straight apostrophes, as the design has it.
const VERDICT_NOTE =
  "The E2E service reports each run on its own. It doesn't compare a baseline with a candidate, and the monitor never grades runs itself. Calling this an improvement needs a campaign whose criterion was fixed before the runs."
const VERDICT_NOTE_NOT_COMPARABLE =
  "Runs that aren't comparable can't support an improvement claim, even if a campaign is run on them. Shown for reference only."
const MEASURES_NOTE =
  "Averages over the runs that reported each value. Missing cost stays missing, never zero. The monitor doesn't subtract, rank or color these numbers."

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

function RunHead({ label, run }: { label: string; run: E2eExecution }) {
  return (
    <span className="eval-ui-val-runhead">
      <span className="eval-ui-val-ink">{label}</span>
      {run.label ? <span>{run.label}</span> : null}
      <span className="eval-ui-val-sub">{run.execution_id}</span>
      <span className="eval-ui-val-sub">
        {run.harness_version ? `harness ${run.harness_version}` : 'harness not reported'}
      </span>
    </span>
  )
}

function Measures({ link, labelledBy }: { link: ValidationLink; labelledBy: string }) {
  return (
    <TableViewport>
      <TableFrame>
        <Table density="compact" aria-labelledby={labelledBy}>
          <TableHeader>
            <TableRow>
              <TableHead scope="col" className="eval-ui-val-rowhead">
                Per execution
              </TableHead>
              <TableHead scope="col" className="eval-ui-val-num">
                <RunHead label="Baseline" run={link.baseline} />
              </TableHead>
              <TableHead scope="col" className="eval-ui-val-num">
                <RunHead label="Candidate" run={link.candidate} />
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {measureRows(link.baseline, link.candidate).map((row) => (
              <TableRow key={row.key}>
                <TableHead scope="row" className="eval-ui-val-rowhead">
                  {row.label}
                </TableHead>
                <TableCell className="eval-ui-val-num">
                  <Cell cell={row.baseline} />
                </TableCell>
                <TableCell className="eval-ui-val-num">
                  <Cell cell={row.candidate} />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableFrame>
    </TableViewport>
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
          <span className="eval-ui-val-mono eval-ui-val-ink">{ids}</span>. The IDs are saved; no verdict is shown until
          the report can be read.
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

export function ValidationPanel({ links }: { links: ValidationLink[] }) {
  const titleId = useId()
  const found = latestLink(links)
  if (!found) return null
  const { latest: link, earlier } = found
  const { comparable, checks } = link.comparability
  const different = mismatches(checks)
  const matching = checks.filter((check) => check.matches)
  const problem = reportProblem(link)
  const scenarios = [...new Set([...scenarioIds(link.baseline), ...scenarioIds(link.candidate)])]

  return (
    <div className="eval-ui-val">
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
          <Badge>
            <span className="eval-ui-val-ring" aria-hidden />
            No E2E verdict
          </Badge>
          {earlier > 0 ? (
            <span className="eval-ui-val-quiet eval-ui-val-earlier">
              {earlier} earlier {earlier === 1 ? 'link' : 'links'}
            </span>
          ) : null}
        </div>

        {problem ? <ReportNotice problem={problem} /> : null}

        <div className="eval-ui-val-section">
          <div className="eval-ui-val-row">
            <span className="eval-ui-val-label">Comparability</span>
            <Badge variant={comparable ? 'ok' : 'alert'}>
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

        {hasMeasures(link) ? (
          <>
            <Measures link={link} labelledBy={titleId} />
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
            attached {formatClock(link.attached_at)} · assets {reportsAvailable(link) ? 'available' : 'unavailable'}
          </span>
        </div>
      </CardHighlight>

      <StatusPanel
        variant="info"
        role="status"
        icon={<Info className={uiClasses.icon} aria-hidden />}
        headline="No E2E verdict"
        detail={comparable ? VERDICT_NOTE : VERDICT_NOTE_NOT_COMPARABLE}
      />
    </div>
  )
}
