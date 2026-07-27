import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import type { HarnessMetricsState } from '@/lib/backend/harness-metrics'
import {
  formatSpan,
  formatUsageValue,
  hasReportedUsage,
  reportedValue,
  type SessionUsage,
} from '@/lib/session-usage'
import { formatTokenCount } from '@/lib/token-estimate'
import { type MetricRow, MetricTable } from './MetricTable'

/**
 * The session metrics view: what this chat consumed, per session and per turn.
 *
 * Purely presentational — props in, nothing fetched, no conversation context.
 * That is deliberate: it keeps this component portable to a right-pane route
 * or a TracesV2 tab as a props change rather than a refactor. Treat an import
 * of `@/lib/iii-client` or `@/lib/conversations-context` here as a bug.
 *
 * The exact/counted/estimated split is structural, not a footnote: which
 * section a number sits in *is* the statement about how much to trust it.
 */

interface SessionMetricsPanelProps {
  usage: SessionUsage
  /** chars/4 heuristic from `lib/token-estimate.ts`. */
  contextEstimate: number
  contextWindow?: number
  tree: HarnessMetricsState | 'loading' | null
  onRetryTree?: () => void
  onViewTraces?: () => void
  showTurnChips: boolean
  onToggleTurnChips: (next: boolean) => void
}

export function SessionMetricsPanel({
  usage,
  contextEstimate,
  contextWindow,
  tree,
  onRetryTree,
  onViewTraces,
  showTurnChips,
  onToggleTurnChips,
}: SessionMetricsPanelProps) {
  const { totals } = usage
  const anyUsage = hasReportedUsage(totals)

  const exactRows: MetricRow[] = [
    {
      label: 'input tokens',
      value: reportedValue(totals, 'input', totals.input),
    },
    {
      label: 'output tokens',
      value: reportedValue(totals, 'output', totals.output),
    },
    {
      label: 'total tokens',
      value: anyUsage ? formatUsageValue(totals.total) : '—',
      note: 'input + output',
    },
    {
      label: 'cache read',
      value: reportedValue(totals, 'cacheRead', totals.cacheRead),
      note: totals.reported.cacheRead === 0 ? 'not reported' : undefined,
      neutral: true,
    },
    {
      label: 'cache write',
      value: reportedValue(totals, 'cacheWrite', totals.cacheWrite),
      note: totals.reported.cacheWrite === 0 ? 'not reported' : undefined,
      neutral: true,
    },
    {
      label: 'reasoning tokens',
      value: reportedValue(totals, 'reasoning', totals.reasoning),
      note: totals.reported.reasoning === 0 ? 'not reported' : undefined,
    },
    {
      label: 'cost',
      value: reportedValue(totals, 'cost', totals.costUsd, 'cost'),
      note: totals.reported.cost === 0 ? 'not reported' : undefined,
    },
  ]

  const countedRows: MetricRow[] = [
    { label: 'turns', value: formatUsageValue(usage.turns.length) },
    {
      label: 'steps',
      value: formatUsageValue(usage.steps),
      note: 'model calls',
    },
    { label: 'function calls', value: formatUsageValue(usage.functionCalls) },
    {
      label: 'function-call errors',
      value: formatUsageValue(usage.functionCallErrors),
      tone: usage.functionCallErrors > 0 ? 'alert' : 'default',
    },
    { label: 'session time', value: formatSpan(usage.durationMs) },
  ]

  const estimatedRows: MetricRow[] = [
    {
      label: 'context in use',
      value: `~${formatTokenCount(contextEstimate)}`,
      note:
        contextWindow && contextWindow > 0
          ? `${Math.round((contextEstimate / contextWindow) * 100)}% of ${formatTokenCount(contextWindow)}`
          : 'context window unknown',
    },
  ]

  if (usage.lastCall) {
    // Raw and un-summed on purpose: `input` means different things across
    // providers (anthropic excludes cached, openai includes it), so any
    // arithmetic here would be wrong for one of them.
    const last = usage.lastCall.usage
    estimatedRows.push({
      label: 'last model call',
      value:
        typeof last.input === 'number'
          ? `${formatUsageValue(last.input)} in`
          : '—',
      note: 'measured prompt, un-summed',
      tone: 'faint',
    })
  }

  return (
    <Tabs defaultValue="session" className="mt-4">
      <TabsList>
        <TabsTrigger value="session">session</TabsTrigger>
        <TabsTrigger value="turns">turns</TabsTrigger>
        <TabsTrigger value="tree">tree</TabsTrigger>
      </TabsList>

      <TabsContent value="session" className="flex flex-col gap-5 pt-4">
        {!anyUsage ? (
          <p className="font-mono text-[11px] text-ink-ghost leading-relaxed">
            {usage.steps === 0
              ? 'no model calls in this session yet.'
              : 'no provider usage recorded for this session — the counted and estimated figures below are still exact.'}
          </p>
        ) : null}
        <MetricTable
          title="exact"
          caption="reported by the provider"
          rows={exactRows}
        />
        <MetricTable
          title="counted"
          caption="from the transcript"
          rows={countedRows}
        />
        <MetricTable
          title="estimated"
          caption="chars ÷ 4 — not provider-reported"
          rows={estimatedRows}
        />
        <div className="flex items-center justify-between gap-3 font-mono text-[11px] text-ink-ghost">
          <span>
            {usage.stepsMissingUsage > 0 && usage.steps > 0
              ? `${usage.stepsMissingUsage} of ${usage.steps} model calls reported no usage`
              : ''}
          </span>
          {onViewTraces ? (
            <button
              type="button"
              onClick={onViewTraces}
              className="text-ink-faint hover:text-ink transition-colors"
            >
              view in traces →
            </button>
          ) : null}
        </div>
        <label className="flex items-center gap-2 font-mono text-[11px] text-ink-faint">
          <input
            type="checkbox"
            checked={showTurnChips}
            onChange={(e) => onToggleTurnChips(e.target.checked)}
            className="accent-accent"
          />
          show per-turn chips in the transcript
        </label>
      </TabsContent>

      <TabsContent value="turns" className="pt-4">
        <TurnsTable usage={usage} />
      </TabsContent>

      <TabsContent value="tree" className="pt-4">
        <TreePanel tree={tree} onRetry={onRetryTree} />
      </TabsContent>
    </Tabs>
  )
}

function TurnsTable({ usage }: { usage: SessionUsage }) {
  const turns = usage.turns.filter((t) => t.steps > 0 || t.functionCalls > 0)
  if (turns.length === 0) {
    return (
      <p className="font-mono text-[11px] text-ink-ghost">
        no completed turns yet.
      </p>
    )
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full font-mono text-[12px]">
        <thead>
          <tr className="text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
            <th className="text-left font-normal py-1.5">turn</th>
            <th className="text-right font-normal py-1.5">steps</th>
            <th className="text-right font-normal py-1.5">tokens</th>
            <th className="text-right font-normal py-1.5">cost</th>
            <th className="text-right font-normal py-1.5">time</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-rule-2 border-t border-rule-2">
          {turns.map((turn, i) => (
            <tr key={turn.turnId}>
              <td className="py-1.5 text-ink-faint">
                {turn.turnId.startsWith('local:') ? `#${i + 1}` : turn.turnId}
                {turn.streaming ? (
                  <span className="text-accent"> · running</span>
                ) : null}
              </td>
              <td className="py-1.5 text-right tabular-nums text-ink-faint">
                {turn.steps}
              </td>
              <td className="py-1.5 text-right tabular-nums text-ink">
                {hasReportedUsage(turn.totals)
                  ? formatUsageValue(turn.totals.total)
                  : '—'}
              </td>
              <td className="py-1.5 text-right tabular-nums text-ink">
                {reportedValue(
                  turn.totals,
                  'cost',
                  turn.totals.costUsd,
                  'cost',
                )}
              </td>
              <td className="py-1.5 text-right tabular-nums text-ink-faint">
                {formatSpan(turn.durationMs)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function TreePanel({
  tree,
  onRetry,
}: {
  tree: HarnessMetricsState | 'loading' | null
  onRetry?: () => void
}) {
  if (tree === 'loading' || tree === null) {
    return (
      <p className="font-mono text-[11px] text-ink-ghost">
        {tree === 'loading' ? 'loading…' : 'not loaded.'}
      </p>
    )
  }

  if (tree.status !== 'ok') {
    return (
      <div className="flex flex-col items-start gap-2">
        <p className="font-mono text-[11px] text-ink-ghost leading-relaxed max-w-md">
          {tree.status === 'incomplete'
            ? 'sub-agent and trace totals need every session in the tree to be idle — unavailable while a turn is running.'
            : 'harness::metrics is unavailable.'}
        </p>
        {onRetry ? (
          <button
            type="button"
            onClick={onRetry}
            className="font-mono text-[11px] text-ink-faint hover:text-ink transition-colors border border-rule px-1.5 py-0.5"
          >
            retry
          </button>
        ) : null}
      </div>
    )
  }

  const { totals, by_session, traces } = tree.metrics
  const num = (v: number | null | undefined, kind?: 'cost') =>
    typeof v === 'number'
      ? formatUsageValue(v, kind === 'cost' ? 'cost' : 'number')
      : '—'

  const rows: MetricRow[] = [
    { label: 'sessions', value: formatUsageValue(totals.sessions) },
    {
      label: 'steps',
      value: formatUsageValue(totals.turns),
      note: 'model calls, whole tree',
    },
    { label: 'input tokens', value: num(totals.input_tokens) },
    { label: 'output tokens', value: num(totals.output_tokens) },
    {
      label: 'cache read',
      value: num(totals.cache_read_tokens),
      neutral: true,
    },
    {
      label: 'cache write',
      value: num(totals.cache_write_tokens),
      neutral: true,
    },
    { label: 'reasoning tokens', value: num(totals.reasoning_tokens) },
    { label: 'cost', value: num(totals.cost_usd, 'cost') },
    { label: 'function calls', value: formatUsageValue(totals.function_calls) },
    {
      label: 'function-call errors',
      value: formatUsageValue(totals.function_call_errors),
      tone: totals.function_call_errors > 0 ? 'alert' : 'default',
    },
  ]

  const traceRows: MetricRow[] = traces
    ? [
        { label: 'traces', value: formatUsageValue(traces.trace_count) },
        { label: 'spans', value: formatUsageValue(traces.span_count) },
        {
          label: 'error spans',
          value: formatUsageValue(traces.error_span_count),
          tone: traces.error_span_count > 0 ? 'alert' : 'default',
        },
        {
          label: 'trace duration',
          value: formatUsageValue(traces.duration_ms, 'duration'),
        },
      ]
    : []

  return (
    <div className="flex flex-col gap-5">
      <MetricTable
        title="tree totals"
        caption="this session and every sub-agent it spawned"
        rows={rows}
      />
      {traceRows.length > 0 ? (
        <MetricTable
          title="traces"
          caption="from the engine span exporter — cleared on restart"
          rows={traceRows}
        />
      ) : null}
      {by_session.length > 1 ? (
        <MetricTable
          title="by session"
          rows={by_session.map((s) => ({
            label: `${'  '.repeat(s.depth)}${s.session_id}`,
            value: num(
              typeof s.input_tokens === 'number' &&
                typeof s.output_tokens === 'number'
                ? s.input_tokens + s.output_tokens
                : null,
            ),
            note: `${s.turns} calls`,
          }))}
        />
      ) : null}
    </div>
  )
}
