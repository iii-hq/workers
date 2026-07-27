import { useEffect, useMemo, useState } from 'react'
import { Badge, Button, JsonHighlight, Markdown, Skeleton, StatusPanel } from '@iii-dev/console-ui'
import { formatDate, formatMetric, shortId, StatusBadge } from '../components'
import { isTerminal } from '../api'
import type {
  AggregateDelta,
  EvalResultResponse,
  EvalRun,
  EvalStatusResponse,
  EvalSummary,
  VariantAggregate,
} from '../types'

export function EvaluationDetail({
  summary,
  status,
  result,
  loading,
  error,
  actionPending,
  onCancel,
  onDelete,
  onRerun,
}: {
  summary: EvalSummary
  status: EvalStatusResponse | null
  result: EvalResultResponse | null
  loading: boolean
  error: string | null
  actionPending: boolean
  onCancel: () => void
  onDelete: () => void
  onRerun: (reverseOrder: boolean) => void
}) {
  const currentStatus = status?.status ?? result?.status ?? summary.status
  const progress = status ?? summary
  const report = result?.report
  const request = result?.request
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1_200)
    return () => window.clearTimeout(timer)
  }, [copied])

  const copyEvaluationId = async () => {
    try {
      await copyText(summary.evaluation_id)
      setCopied(true)
    } catch {}
  }

  return (
    <div className="eval-ui-detail">
      <div className="eval-ui-detail-head">
        <div>
          <div className="eval-ui-title-row">
            <h1>
              {summary.control_label ?? 'control'} vs {summary.treatment_label ?? 'treatment'}
            </h1>
            <StatusBadge status={currentStatus} />
            {report?.eligible !== undefined ? (
              <Badge variant={report.eligible ? 'accent' : 'warn'}>
                {report.eligible ? 'candidate eligible' : 'candidate not eligible'}
              </Badge>
            ) : null}
          </div>
          <p>
            {summary.model}
            {summary.provider ? ` · ${summary.provider}` : ''} · {summary.dimension.replace('_', ' ')}
          </p>
          <div className="eval-ui-id">
            <span className="eval-ui-label">evaluation id</span>
            <code title={summary.evaluation_id}>{shortId(summary.evaluation_id)}</code>
            <button
              type="button"
              onClick={() => void copyEvaluationId()}
              aria-label={copied ? 'copied' : 'copy evaluation id'}
              title={copied ? 'copied' : 'copy'}
            >
              {copied ? <CheckIcon /> : <CopyIcon />}
            </button>
            <span className="eval-ui-sr-only" aria-live="polite">
              {copied ? 'Evaluation ID copied to clipboard.' : ''}
            </span>
          </div>
        </div>
        <div className="eval-ui-actions">
          {!isTerminal(currentStatus) ? (
            <Button variant="ghost" size="sm" onClick={onCancel} disabled={actionPending}>
              cancel
            </Button>
          ) : (
            <>
              <Button variant="ghost" size="sm" onClick={() => onRerun(false)} disabled={actionPending}>
                run again
              </Button>
              <Button variant="ghost" size="sm" onClick={() => onRerun(true)} disabled={actionPending}>
                run reversed
              </Button>
              <Button variant="ghost" size="sm" onClick={onDelete} disabled={actionPending}>
                delete
              </Button>
            </>
          )}
        </div>
      </div>

      <Progress
        terminal={progress.terminal_runs}
        total={progress.total_runs}
        passed={progress.passed_runs}
        failed={progress.failed_runs}
      />

      {error ? (
        <StatusPanel variant="alert" headline="evaluation details unavailable" detail={error} />
      ) : loading && !result ? (
        <div className="eval-ui-loading">
          <Skeleton />
          <Skeleton />
          <Skeleton />
        </div>
      ) : null}

      {!isTerminal(currentStatus) ? (
        <StatusPanel
          variant="info"
          headline={`evaluation ${currentStatus}`}
          detail={
            status?.active
              ? `${status.active.role} run ${status.active.iteration} is active (${shortId(status.active.session_id)}).`
              : 'waiting for the next durable evaluation step.'
          }
        />
      ) : currentStatus !== 'completed' && !report ? (
        <StatusPanel
          variant={currentStatus === 'failed' ? 'alert' : 'warn'}
          headline={`evaluation ${currentStatus}`}
          detail={status?.error ?? summary.error ?? 'no report was produced.'}
        />
      ) : null}

      {request ? <Definition result={result} /> : null}
      {result ? <Report result={result} activeRunId={status?.active?.run_id} /> : null}
    </div>
  )
}

function Progress({
  terminal,
  total,
  passed,
  failed,
}: {
  terminal: number
  total: number
  passed: number
  failed: number
}) {
  const percentage = total > 0 ? Math.min(100, (terminal / total) * 100) : 0
  const notEvaluated = Math.max(0, terminal - passed - failed)
  return (
    <div className="eval-ui-progress">
      <div className="eval-ui-progress-meta">
        <span>
          {terminal}/{total} runs complete
        </span>
        <span>
          {passed} met criteria · {failed} missed criteria
          {notEvaluated > 0 ? ` · ${notEvaluated} not evaluated` : ''}
        </span>
      </div>
      <div className="eval-ui-progress-track">
        <span style={{ width: `${percentage}%` }} />
      </div>
    </div>
  )
}

function Definition({ result }: { result: EvalResultResponse }) {
  const { request } = result
  const varied = request.dimension === 'prompt' ? 'prompt' : 'system prompt'
  const shared = request.dimension === 'prompt' ? request.control.system_prompt : request.control.prompt
  return (
    <section className="eval-ui-panel">
      <div className="eval-ui-panel-title">definition</div>
      <div className="eval-ui-definition-meta">
        <span>{request.runs} runs per variant</span>
        <span>{orderLabel(request.execution_order)}</span>
        {request.source_evaluation_id ? <span>rerun of {shortId(request.source_evaluation_id)}</span> : null}
        <span>{request.evaluator?.function_id ?? 'no success criteria'}</span>
        <span>
          {request.limits.execution.max_total_tokens === undefined
            ? 'no total-token limit'
            : `max ${formatMetric(request.limits.execution.max_total_tokens)} tokens`}
        </span>
        {request.limits.execution.max_cost_usd !== undefined ? (
          <span>max {formatMetric(request.limits.execution.max_cost_usd, 'cost')}</span>
        ) : null}
      </div>
      <div className="eval-ui-order">
        <span className="eval-ui-label">execution sequence</span>
        <span className="eval-ui-hint">Runs execute from left to right. A is the baseline; B is the candidate.</span>
        <div className="eval-ui-order-list">
          {result.progress.effective_execution_order.map((value, index) => {
            const step = executionStep(
              value,
              request.control.label ?? 'baseline',
              request.treatment.label ?? 'candidate',
            )
            return (
              <div className="eval-ui-order-item-wrap" key={`${value}-${index}`}>
                {index > 0 ? <span className="eval-ui-order-arrow" aria-hidden="true">→</span> : null}
                <div className="eval-ui-order-item" title={value}>
                  <strong>{step.marker}</strong>
                  <span>{step.label}</span>
                </div>
              </div>
            )
          })}
        </div>
      </div>
      <div className="eval-ui-shared">
        <span>{request.dimension === 'prompt' ? 'shared system prompt' : 'shared prompt'}</span>
        <pre>
          {request.dimension === 'prompt' && shared === null
            ? '— no system prompt —'
            : shared || '— built-in system prompt —'}
        </pre>
      </div>
      <div className="eval-ui-variants">
        {[
          { marker: 'A', role: 'baseline', variant: request.control },
          { marker: 'B', role: 'candidate', variant: request.treatment },
        ].map(({ marker, role, variant }) => {
          return (
            <div className="eval-ui-variant readonly" key={role}>
              <div className="eval-ui-variant-title">
                {marker} · {variant.label ?? role}
              </div>
              <span className="eval-ui-label">{varied}</span>
              <pre>
                {request.dimension === 'prompt'
                  ? variant.prompt
                  : variant.system_prompt === null
                    ? '— no system prompt —'
                    : variant.system_prompt || '— built-in system prompt —'}
              </pre>
            </div>
          )
        })}
      </div>
      {result.report ? <ArtifactHashes report={result.report} /> : null}
    </section>
  )
}

function Report({ result, activeRunId }: { result: EvalResultResponse; activeRunId?: string }) {
  const report = result.report
  const progress = result.progress
  return (
    <section className="eval-ui-panel">
      <div className="eval-ui-panel-title">report</div>
      <div className="eval-ui-report-section">
        <div className="eval-ui-label">runs</div>
        <Runs
          runs={progress.runs}
          controlLabel={result.request.control.label ?? 'baseline'}
          treatmentLabel={result.request.treatment.label ?? 'candidate'}
          activeRunId={activeRunId}
          orderSensitive={progress.order_sensitivity.detected}
        />
      </div>
      <div className="eval-ui-report-section">
        <div className="eval-ui-label">summary</div>
        <div className="eval-ui-report-note">A is the baseline. B is the candidate. Delta is B − A.</div>
        <AggregateTable
          control={progress.control_aggregate}
          treatment={progress.treatment_aggregate}
          delta={progress.delta}
          controlLabel={result.request.control.label ?? 'control'}
          treatmentLabel={result.request.treatment.label ?? 'treatment'}
        />
        {progress.order_sensitivity.detected ? (
          <StatusPanel
            variant="warn"
            headline="order-sensitive result"
            detail="At least one variant has a different pass rate when it runs first versus second."
          />
        ) : null}
      </div>
      {report ? (
        <div className="eval-ui-report-foot">
          completed {formatDate(report.completed_at)} · efficiency metrics are descriptive and do not select a winner.
        </div>
      ) : (
        <div className="eval-ui-report-foot">Summary includes completed runs only and updates while the evaluation runs.</div>
      )}
    </section>
  )
}

interface MetricRow {
  label: string
  control: keyof VariantAggregate
  delta: keyof AggregateDelta
  format?: 'number' | 'percent' | 'duration' | 'cost'
  higherIsBetter?: boolean
}

const METRICS: MetricRow[] = [
  {
    label: 'pass rate',
    control: 'pass_rate',
    delta: 'pass_rate',
    format: 'percent',
    higherIsBetter: true,
  },
  {
    label: 'median score',
    control: 'median_score',
    delta: 'median_score',
    higherIsBetter: true,
  },
  {
    label: 'wall time',
    control: 'median_wall_time_ms',
    delta: 'median_wall_time_ms',
    format: 'duration',
  },
  {
    label: 'total tokens',
    control: 'median_total_tokens',
    delta: 'median_total_tokens',
  },
  {
    label: 'reasoning tokens',
    control: 'median_reasoning_tokens',
    delta: 'median_reasoning_tokens',
  },
  {
    label: 'cost',
    control: 'median_cost_usd',
    delta: 'median_cost_usd',
    format: 'cost',
  },
  {
    label: 'function calls',
    control: 'median_function_calls',
    delta: 'median_function_calls',
  },
  {
    label: 'function-call errors',
    control: 'median_function_call_errors',
    delta: 'median_function_call_errors',
  },
  {
    label: 'traces',
    control: 'median_trace_count',
    delta: 'median_trace_count',
  },
  {
    label: 'spans',
    control: 'median_span_count',
    delta: 'median_span_count',
  },
  {
    label: 'error spans',
    control: 'median_error_span_count',
    delta: 'median_error_span_count',
  },
  {
    label: 'trace duration',
    control: 'median_trace_duration_ms',
    delta: 'median_trace_duration_ms',
    format: 'duration',
  },
]

function AggregateTable({
  control,
  treatment,
  delta,
  controlLabel,
  treatmentLabel,
}: {
  control: VariantAggregate
  treatment: VariantAggregate
  delta: AggregateDelta
  controlLabel: string
  treatmentLabel: string
}) {
  return (
    <div className="eval-ui-metrics">
      <div className="eval-ui-success-counts">
        <span>A · {control.passed}/{control.evaluated_runs} passed · {control.runs} completed</span>
        <span>B · {treatment.passed}/{treatment.evaluated_runs} passed · {treatment.runs} completed</span>
      </div>
      <div className="eval-ui-metric header">
        <span>metric</span>
        <span>A · {controlLabel}</span>
        <span>B · {treatmentLabel}</span>
        <span>B − A</span>
      </div>
      {METRICS.map((metric) => {
        const controlValue =
          metric.control === 'pass_rate' && control.evaluated_runs === 0
            ? undefined
            : (control[metric.control] as number | undefined)
        const treatmentValue =
          metric.control === 'pass_rate' && treatment.evaluated_runs === 0
            ? undefined
            : (treatment[metric.control] as number | undefined)
        const deltaValue =
          metric.control === 'pass_rate' && (control.evaluated_runs === 0 || treatment.evaluated_runs === 0)
            ? undefined
            : (delta[metric.delta] as number | undefined)
        const positive =
          deltaValue !== undefined && deltaValue !== 0 && (metric.higherIsBetter ? deltaValue > 0 : deltaValue < 0)
        const negative =
          deltaValue !== undefined && deltaValue !== 0 && (metric.higherIsBetter ? deltaValue < 0 : deltaValue > 0)
        return (
          <div className="eval-ui-metric" key={metric.label}>
            <span>{metric.label}</span>
            <span>{formatMetric(controlValue, metric.format)}</span>
            <span>{formatMetric(treatmentValue, metric.format)}</span>
            <span className={positive ? 'positive' : negative ? 'negative' : undefined}>
              {deltaValue !== undefined && deltaValue > 0 ? '+' : ''}
              {formatMetric(deltaValue, metric.format)}
            </span>
          </div>
        )
      })}
    </div>
  )
}

function Runs({
  runs,
  controlLabel,
  treatmentLabel,
  activeRunId,
  orderSensitive,
}: {
  runs: EvalRun[]
  controlLabel: string
  treatmentLabel: string
  activeRunId?: string
  orderSensitive: boolean
}) {
  const [selectedId, setSelectedId] = useState<string | null>(runs[0]?.run_id ?? null)
  useEffect(() => {
    if (!runs.some((run) => run.run_id === selectedId)) {
      setSelectedId(runs[0]?.run_id ?? null)
    }
  }, [runs, selectedId])
  const selected = useMemo(() => runs.find((run) => run.run_id === selectedId) ?? null, [runs, selectedId])
  const iterations = useMemo(() => {
    const values = new Map<number, { control?: EvalRun; treatment?: EvalRun }>()
    for (const run of runs) {
      const pair = values.get(run.iteration) ?? {}
      pair[run.role] = run
      values.set(run.iteration, pair)
    }
    return [...values.entries()].sort(([left], [right]) => left - right)
  }, [runs])
  return (
    <div className="eval-ui-runs">
      <div className="eval-ui-run-pairs">
        {iterations.map(([iteration, pair]) => (
          <div className={`eval-ui-run-pair${orderSensitive ? ' order-sensitive' : ''}`} key={iteration}>
            <div className="eval-ui-run-pair-head">iteration {iteration}</div>
            <div className="eval-ui-run-pair-grid">
              {pair.control ? (
                <RunCard
                  run={pair.control}
                  marker="A"
                  label={controlLabel}
                  active={pair.control.run_id === activeRunId}
                  selected={pair.control.run_id === selectedId}
                  onSelect={setSelectedId}
                />
              ) : null}
              {pair.treatment ? (
                <RunCard
                  run={pair.treatment}
                  marker="B"
                  label={treatmentLabel}
                  active={pair.treatment.run_id === activeRunId}
                  selected={pair.treatment.run_id === selectedId}
                  onSelect={setSelectedId}
                />
              ) : null}
            </div>
          </div>
        ))}
      </div>
      {selected ? <RunDetail run={selected} /> : null}
    </div>
  )
}

function RunCard({
  run,
  marker,
  label,
  active,
  selected,
  onSelect,
}: {
  run: EvalRun
  marker: string
  label: string
  active: boolean
  selected: boolean
  onSelect: (id: string) => void
}) {
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (!active) return
    const timer = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(timer)
  }, [active])
  const elapsed = active ? Math.max(0, now - run.started_at) : run.benchmark?.wall_time_ms
  return (
    <button
      type="button"
      className={`eval-ui-run-card${active ? ' running' : ''}${selected ? ' selected' : ''}`}
      onClick={() => onSelect(run.run_id)}
    >
      <span className="eval-ui-run-card-title">{marker} · {label}</span>
      <span>{runStatus(run)}</span>
      <span>{run.pair_position === 1 ? 'ran first' : 'ran second'} · #{run.execution_position}</span>
      <span>{formatMetric(run.benchmark?.total_tokens)} tokens · {formatMetric(elapsed, 'duration')}</span>
    </button>
  )
}

function runStatus(run: EvalRun): string {
  if (run.passed) return 'met criteria'
  if (run.passed === false) return 'missed criteria'
  if (run.status === 'completed') return 'not evaluated'
  return run.status
}

function RunDetail({ run }: { run: EvalRun }) {
  const failures = run.failures ?? []
  return (
    <div className="eval-ui-run-detail">
      <div className="eval-ui-definition-meta">
        <span>{run.session_id}</span>
        <span>{run.evaluation?.reason ?? 'not evaluated'}</span>
      </div>
      {failures.length > 0 ? (
        <div className="eval-ui-failures">
          {failures.map((failure, index) => (
            <div key={`${failure.phase}-${index}`}>
              <strong>{failure.phase}</strong>
              {failure.function_id ? ` · ${failure.function_id}` : ''} — {failure.message}
            </div>
          ))}
        </div>
      ) : null}
      <div className="eval-ui-label">output</div>
      <Markdown className="eval-ui-markdown-output">{outputAsMarkdown(run.output)}</Markdown>
      {run.evaluation?.details !== undefined ? (
        <>
          <div className="eval-ui-label">evaluator details</div>
          <JsonHighlight code={JSON.stringify(run.evaluation.details, null, 2)} wrap />
        </>
      ) : null}
    </div>
  )
}

function orderLabel(order: EvalResultResponse['request']['execution_order']): string {
  return order === 'balanced_treatment_first' ? 'balanced · B first' : 'balanced · A first'
}

function executionStep(
  value: string,
  controlLabel: string,
  treatmentLabel: string,
): { marker: string; label: string } {
  const [, role] = value.split(':')
  if (role === 'control') {
    return { marker: 'A', label: controlLabel }
  }
  if (role === 'treatment') {
    return { marker: 'B', label: treatmentLabel }
  }
  return { marker: '?', label: role || value }
}

function CopyIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </svg>
  )
}

function CheckIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
      <path d="m20 6-11 11-5-5" />
    </svg>
  )
}

async function copyText(value: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value)
    return
  }
  const input = document.createElement('textarea')
  input.value = value
  input.style.position = 'fixed'
  input.style.opacity = '0'
  document.body.appendChild(input)
  input.select()
  const copied = document.execCommand('copy')
  input.remove()
  if (!copied) throw new Error('copy command failed')
}

function ArtifactHashes({ report }: { report: NonNullable<EvalResultResponse['report']> }) {
  const values = [
    ['A prompt', report.control.prompt_sha256],
    ['A system', report.control.system_prompt_sha256 ?? 'none'],
    ['B prompt', report.treatment.prompt_sha256],
    ['B system', report.treatment.system_prompt_sha256 ?? 'none'],
    ['model', report.shared_artifacts.model_sha256],
    ['functions', report.shared_artifacts.function_policy_sha256],
    ['limits', report.shared_artifacts.limits_sha256],
    ['output', report.shared_artifacts.output_sha256],
    ['evaluator', report.evaluator?.arguments_sha256 ?? 'none'],
  ]
  return (
    <details className="eval-ui-hashes">
      <summary>comparison hashes</summary>
      <div>
        {values.map(([label, hash]) => (
          <code key={label}>
            <span>{label}</span>
            {hash}
          </code>
        ))}
      </div>
    </details>
  )
}

function outputAsMarkdown(output: EvalRun['output']): string {
  if (typeof output === 'string') return output

  return `\`\`\`json\n${JSON.stringify(output ?? null, null, 2)}\n\`\`\``
}
