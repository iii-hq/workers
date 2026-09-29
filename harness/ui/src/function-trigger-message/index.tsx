/**
 * Injected function-trigger message renderer for `harness::metrics` —
 * registered through `host.functionTriggers`, so it dispatches BEFORE the
 * console's built-in families and replaces the raw JSON card in chat and
 * traces with a totals strip plus a compact per-session table (depth-indented
 * ids, turn/token/cost columns, and a mini context-usage bar when the
 * session carries a snapshot). Anything unparseable returns null and falls
 * through to the default rendering.
 */

import {
  Badge,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  MetaRow,
  type MetaRowItem,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { formatCost, formatTokens } from '../lib/format'
import {
  type MetricsResponse,
  type SessionUsage,
  parseMetrics,
} from '../lib/metrics'
import { TONE_COLOR, toneFor } from '../lib/tone'

const METRICS_ID = 'harness::metrics'

function totalsItems(metrics: MetricsResponse): MetaRowItem[] {
  const totals = metrics.totals
  const items: MetaRowItem[] = [
    { label: 'sessions', value: String(totals.sessions ?? 0) },
    { label: 'turns', value: String(totals.turns ?? 0) },
  ]
  if (totals.input_tokens != null) items.push({ label: 'in', value: formatTokens(totals.input_tokens) })
  if (totals.output_tokens != null) items.push({ label: 'out', value: formatTokens(totals.output_tokens) })
  if (totals.cache_read_tokens != null)
    items.push({ label: 'cache', value: formatTokens(totals.cache_read_tokens) })
  if (totals.cost_usd != null) items.push({ label: 'cost', value: formatCost(totals.cost_usd) })
  return items
}

function MiniUsageBar({ total, usable }: { total: number; usable: number }) {
  if (usable <= 0) return null
  const ratio = Math.min(1, total / usable)
  const pct = Math.round(ratio * 100)
  const color = TONE_COLOR[toneFor(ratio)]
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
    <TableRow>
      <TableCell className="harness-ui-sid">
        <span style={{ paddingLeft: `${(row.depth ?? 0) * 12}px` }}>
          {row.session_id}
        </span>
      </TableCell>
      <TableCell className="harness-ui-num">{row.turns ?? 0}</TableCell>
      <TableCell className="harness-ui-num">
        {row.input_tokens != null ? formatTokens(row.input_tokens) : '—'}
      </TableCell>
      <TableCell className="harness-ui-num">
        {row.output_tokens != null ? formatTokens(row.output_tokens) : '—'}
      </TableCell>
      <TableCell className="harness-ui-num">
        {row.cost_usd != null ? formatCost(row.cost_usd) : '—'}
      </TableCell>
      <TableCell>
        {row.context ? (
          <MiniUsageBar total={row.context.total} usable={row.context.usable} />
        ) : null}
      </TableCell>
    </TableRow>
  )
}

function MetricsCard({ metrics }: { metrics: MetricsResponse }) {
  return (
    <div className="harness-ui-msg">
      <MetaRow items={totalsItems(metrics)}>
        {metrics.complete === false ? <Badge variant="warn">partial</Badge> : null}
      </MetaRow>
      {metrics.by_session.length > 0 ? (
        <TableViewport className="harness-ui-table-wrap">
          <TableFrame>
            <Table density="compact">
              <TableHeader>
                <TableRow>
                  <TableHead>session</TableHead>
                  <TableHead className="harness-ui-num">turns</TableHead>
                  <TableHead className="harness-ui-num">in</TableHead>
                  <TableHead className="harness-ui-num">out</TableHead>
                  <TableHead className="harness-ui-num">cost</TableHead>
                  <TableHead>ctx</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {metrics.by_session.map((row) => (
                  <SessionRow key={row.session_id} row={row} />
                ))}
              </TableBody>
            </Table>
          </TableFrame>
        </TableViewport>
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
