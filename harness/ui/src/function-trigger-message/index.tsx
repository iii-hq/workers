/**
 * Injected function-trigger message renderer for `harness::metrics` —
 * registered through `host.functionTriggers`, so it dispatches BEFORE the
 * console's built-in families and replaces the raw JSON card in chat and
 * traces with a totals row plus a compact per-session table (depth-indented
 * ids, turn/token/cost columns, and a mini context-usage bar when the
 * session carries a snapshot). Anything unparseable returns null and falls
 * through to the default rendering.
 */

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
} from '@iii-dev/console-ui'
import { formatCost, formatTokens } from '../lib/format'
import {
  type MetricsResponse,
  type SessionUsage,
  parseMetrics,
} from '../lib/metrics'

const METRICS_ID = 'harness::metrics'

function TotalsChip({ label, value }: { label: string; value: string }) {
  return (
    <span className="harness-ui-chip-stat">
      <span className="k">{label} </span>
      {value}
    </span>
  )
}

function TotalsRow({ metrics }: { metrics: MetricsResponse }) {
  const totals = metrics.totals
  return (
    <div className="harness-ui-totals">
      <TotalsChip label="sessions" value={String(totals.sessions ?? 0)} />
      <TotalsChip label="turns" value={String(totals.turns ?? 0)} />
      {totals.input_tokens != null ? (
        <TotalsChip label="in" value={formatTokens(totals.input_tokens)} />
      ) : null}
      {totals.output_tokens != null ? (
        <TotalsChip label="out" value={formatTokens(totals.output_tokens)} />
      ) : null}
      {totals.cache_read_tokens != null ? (
        <TotalsChip label="cache" value={formatTokens(totals.cache_read_tokens)} />
      ) : null}
      {totals.cost_usd != null ? (
        <TotalsChip label="cost" value={formatCost(totals.cost_usd)} />
      ) : null}
    </div>
  )
}

function MiniUsageBar({ total, usable }: { total: number; usable: number }) {
  if (usable <= 0) return null
  const ratio = Math.min(1, total / usable)
  const pct = Math.round(ratio * 100)
  const color =
    ratio >= 0.9
      ? 'var(--color-alert)'
      : ratio >= 0.75
        ? 'var(--color-warn)'
        : 'var(--color-accent)'
  return (
    <span
      className="harness-ui-mini-bar"
      title={`${formatTokens(total)}/${formatTokens(usable)} (${pct}%)`}
    >
      <span
        className="harness-ui-mini-fill"
        style={{ width: `${pct}%`, background: color }}
      />
    </span>
  )
}

function SessionRow({ row }: { row: SessionUsage }) {
  return (
    <tr>
      <td className="harness-ui-sid">
        <span style={{ paddingLeft: `${(row.depth ?? 0) * 12}px` }}>
          {row.session_id}
        </span>
      </td>
      <td className="harness-ui-num">{row.turns ?? 0}</td>
      <td className="harness-ui-num">
        {row.input_tokens != null ? formatTokens(row.input_tokens) : '—'}
      </td>
      <td className="harness-ui-num">
        {row.output_tokens != null ? formatTokens(row.output_tokens) : '—'}
      </td>
      <td className="harness-ui-num">
        {row.cost_usd != null ? formatCost(row.cost_usd) : '—'}
      </td>
      <td>
        {row.context ? (
          <MiniUsageBar total={row.context.total} usable={row.context.usable} />
        ) : null}
      </td>
    </tr>
  )
}

function MetricsCard({ metrics }: { metrics: MetricsResponse }) {
  return (
    <div className="harness-ui-msg">
      <div className="harness-ui-msg-head">
        <span className="harness-ui-pill">metrics</span>
        {metrics.complete === false ? (
          <span className="harness-ui-partial">partial</span>
        ) : null}
        <span className="harness-ui-msg-tag">harness ui</span>
      </div>
      <TotalsRow metrics={metrics} />
      {metrics.by_session.length > 0 ? (
        <div className="harness-ui-table-wrap">
          <table className="harness-ui-table">
            <thead>
              <tr>
                <th>session</th>
                <th className="harness-ui-num">turns</th>
                <th className="harness-ui-num">in</th>
                <th className="harness-ui-num">out</th>
                <th className="harness-ui-num">cost</th>
                <th>ctx</th>
              </tr>
            </thead>
            <tbody>
              {metrics.by_session.map((row) => (
                <SessionRow key={row.session_id} row={row} />
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </div>
  )
}

export function createMetricsRenderer(): FunctionTriggerRenderer {
  return {
    id: 'harness/page.js#metrics',
    isMatch: (functionId) => functionId === METRICS_ID,
    tryRender: (message: FunctionTriggerMessage) => {
      const metrics = parseMetrics(message.output)
      if (!metrics) return null
      return <MetricsCard metrics={metrics} />
    },
  }
}
