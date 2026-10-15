/**
 * The `context` session chip — a live view of the session's context window
 * matching the console's ContextUsage aesthetic (`ctx` label, bar, `12.3k/200k`
 * counts). Hydrates from the stored snapshot (`state::get` on
 * `harness_context/<session id>`) on mount and per session change; stays live
 * over the state worker's own `state` trigger for that key (Message-path
 * binding, GC'd with the tab), which fires on every generate step. Click opens
 * the breakdown — a `Dialog` on desktop, a `BottomSheet` on phones — with the
 * window (stacked bar, legend, last step), the conversation's usage from
 * `harness::metrics` (plus a with-sub-agents column when it spawned any),
 * where that subtree's cost went (with a page listing every sub-agent), and
 * the session id with copy.
 *
 * The chip answers to the PANE, not the viewport: under a 30rem-wide chat
 * pane (a phone, or a narrow split) it drops to its counts — the bar and the
 * `ctx` eyebrow hide (styles.css, `@container`), `12.3k/200k` and the caret
 * stay. `data-has-counts` marks the states that have counts to fall back to;
 * the waiting state (`ctx —`) keeps its eyebrow.
 */

import {
  Badge,
  BottomSheet,
  BottomSheetContent,
  BottomSheetTrigger,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
  type Host,
  IconButton,
  KeyCombo,
  type LiveAnnouncement,
  ListItem,
  LiveRegion,
  SearchField,
  SegmentedControl,
  Skeleton,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import {
  ArrowLeft,
  ArrowUpRight,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  CornerDownRight,
  Info,
  TriangleAlert,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { formatCost, formatTokens } from '../lib/format'
import { COLOR_FREE, categories, type LegendEntry, legendRows, percentOf } from '../lib/legend'
import {
  type ContextSnapshot,
  type ConversationTree,
  cacheHitPct,
  conversationTree,
  filterGroups,
  type GroupSort,
  isSnapshot,
  type MetricsResponse,
  parseMetrics,
  type SessionUsage,
  type SubAgentGroup,
  type SummedKey,
  sortGroups,
  subAgentGroups,
  sumUsage,
  treeTotals,
  type UsageCounters,
  type UsageTotals,
} from '../lib/metrics'
import { TONE_COLOR, toneFor } from '../lib/tone'

/** Per-tab handler id (host.iii.on namespaces it `::<browserId>`). */
const STATE_FN = 'iii::harness-ui::ctx-state'
const METRICS_FN = 'harness::metrics'
/** `harness::metrics` re-reads every transcript of the tree: a 179-session
    tree took over 30 s. Past this the skeleton gives way to the error and its
    retry. */
const METRICS_TIMEOUT_MS = 90_000
/** Phone chrome only (sheet vs dialog), the console's one viewport breakpoint. */
const PHONE_QUERY = '(max-width: 639px)'

export interface SessionChipProps {
  sessionId: string
  modelId?: string
  contextWindow?: number
}

/** The message a `state` function trigger delivers per write (the state
    worker's own streaming trigger type — no polling anywhere). */
interface StateEvent {
  type?: string
  event_type?: string
  scope?: string
  key?: string
  new_value?: unknown
}

function usePhone(): boolean {
  const [phone, setPhone] = useState(
    () => typeof window !== 'undefined' && window.matchMedia?.(PHONE_QUERY).matches === true,
  )
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const media = window.matchMedia(PHONE_QUERY)
    const update = () => setPhone(media.matches)
    update()
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [])
  return phone
}

/**
 * `harness::metrics` for the open dialog. `data` is the last good answer: it
 * stays on screen while a refresh is in flight and when one fails. `key` is
 * the session and turn it answered for.
 */
type MetricsState =
  | { status: 'idle' }
  | { status: 'loading'; data?: MetricsResponse; key?: string }
  | { status: 'ready'; data: MetricsResponse; key: string }
  | { status: 'error'; error: string; data?: MetricsResponse; key?: string }

/**
 * Usage comes from `harness::metrics`, which re-reads every transcript of the
 * session tree — too heavy for every generate step, or for every open. It is
 * read when the breakdown opens on a turn it has no answer for yet, and again
 * when a new turn lands while it stays open; Retry forces a read.
 */
function useConversationMetrics(
  host: Host,
  sessionId: string,
  open: boolean,
  turnId: string | undefined,
) {
  const [state, setState] = useState<MetricsState>({ status: 'idle' })
  const [attempt, setAttempt] = useState(0)
  const forced = useRef(0)
  // Read by the fetch effect to skip a turn it already has an answer for.
  const current = useRef(state)
  current.current = state
  const key = `${sessionId}\n${turnId ?? ''}`
  useEffect(() => {
    setState({ status: 'idle' })
  }, [sessionId])
  useEffect(() => {
    if (!open) return
    const force = forced.current !== attempt
    forced.current = attempt
    const known = current.current
    if (known.status === 'ready' && known.key === key && !force) return
    let cancelled = false
    setState((prev) =>
      prev.status === 'idle'
        ? { status: 'loading' }
        : { status: 'loading', data: prev.data, key: prev.key },
    )
    host.iii
      .trigger(METRICS_FN, { session_id: sessionId }, { timeoutMs: METRICS_TIMEOUT_MS })
      .then((value) => {
        if (cancelled) return
        const data = parseMetrics(value)
        setState((prev) =>
          data
            ? { status: 'ready', data, key }
            : {
                status: 'error',
                error: `${METRICS_FN} returned an unexpected shape`,
                data: prev.status === 'idle' ? undefined : prev.data,
              },
        )
      })
      .catch((err: unknown) => {
        if (cancelled) return
        setState((prev) => ({
          status: 'error',
          error: errorMessage(err),
          data: prev.status === 'idle' ? undefined : prev.data,
        }))
      })
    return () => {
      cancelled = true
    }
  }, [host, sessionId, open, key, attempt])
  return { state, retry: () => setAttempt((n) => n + 1) }
}

function LegendRow({ row, usable }: { row: LegendEntry; usable: number }) {
  return (
    <div
      className="harness-ui-legend-row"
      data-group={row.group ? '' : undefined}
      data-sub={row.sub ? '' : undefined}
    >
      <span
        className="harness-ui-swatch"
        style={row.color ? { background: row.color } : { visibility: 'hidden' }}
      />
      <span className="harness-ui-legend-label">{row.label}</span>
      {row.badge ? <Badge variant="warn">{row.badge}</Badge> : null}
      <span className="harness-ui-legend-val">{formatTokens(row.tokens)}</span>
      <span className="harness-ui-legend-pct">{percentOf(row.tokens, usable)}</span>
    </div>
  )
}

function SectionHead({ id, title, meta }: { id: string; title: string; meta?: ReactNode }) {
  return (
    <div className="harness-ui-section-head">
      <h3 id={id} className="harness-ui-section-title">
        {title}
      </h3>
      {meta ? <span className="harness-ui-section-meta">{meta}</span> : null}
    </div>
  )
}

/** The window: provenance, stacked bar, legend, and the last step's actuals. */
function WindowSection({ snapshot }: { snapshot: ContextSnapshot }) {
  const headingId = useId()
  const usable = snapshot.usable
  const cats = categories(snapshot)
  const usage = snapshot.usage
  const estimated = !snapshot.estimator || snapshot.estimator === 'heuristic'
  const prompt = (usage?.input ?? 0) + (usage?.cache_read ?? 0) + (usage?.cache_write ?? 0)
  const hasActuals = usage != null && (usage.input != null || usage.cache_read != null)
  const hit =
    usage && (usage.cache_read ?? 0) + (usage.cache_write ?? 0) > 0
      ? cacheHitPct({
          turns: 1,
          function_calls: 0,
          function_call_errors: 0,
          input_tokens: usage.input ?? 0,
          cache_read_tokens: usage.cache_read ?? 0,
          cache_write_tokens: usage.cache_write,
        })
      : null
  return (
    <section className="harness-ui-section" aria-labelledby={headingId}>
      <SectionHead
        id={headingId}
        title="Context window"
        meta={
          estimated
            ? `estimate · ${snapshot.estimator ?? 'unknown'}`
            : `exact · ${snapshot.estimator === 'provider' ? 'provider tokenizer' : snapshot.estimator}`
        }
      />
      <div className="harness-ui-stack">
        {cats
          .filter((segment) => segment.tokens > 0)
          .map((segment) => (
            <span
              key={segment.key}
              className="harness-ui-seg"
              style={{
                width: `${usable > 0 ? (segment.tokens / usable) * 100 : 0}%`,
                background: segment.color,
              }}
            />
          ))}
      </div>
      <div className="harness-ui-legend">
        {legendRows(snapshot, cats).map((row) => (
          <LegendRow key={row.key} row={row} usable={usable} />
        ))}
      </div>
      {hasActuals ? (
        <div className="harness-ui-step">
          <span className="harness-ui-step-label">Last step</span>
          {/* `prompt` is the whole prompt the provider read (fresh input plus
              cache read and write), unlike Usage's uncached input. */}
          <span className="harness-ui-step-val">
            {formatTokens(prompt)} prompt · {formatTokens(usage?.output ?? 0)} out
            {hit != null ? (
              <>
                {' · '}
                <span
                  className="harness-ui-step-hit"
                  title={
                    'cache read is billed at a fraction of fresh input; cache write ' +
                    'carries a premium. This is the cached share of the prompt.'
                  }
                >
                  {hit}% cached
                </span>
              </>
            ) : null}
            {usage?.cost_usd != null ? <> · {formatCost(usage.cost_usd)}</> : null}
          </span>
        </div>
      ) : null}
    </section>
  )
}

const tokensOrDash = (n: number | undefined) => (n == null ? '—' : formatTokens(n))
const costOrDash = (n: number | undefined) => (n == null ? '—' : formatCost(n))
/** Grouped the English way, whatever the browser locale: the copy is English. */
const num = (n: number) => n.toLocaleString('en-US')
const plural = (n: number, word: string) => `${num(n)} ${word}${n === 1 ? '' : 's'}`

/** A lower bound — some session left the counter unreported — reads `≥`. */
function Bound({ partial, children }: { partial: boolean; children: ReactNode }) {
  return partial ? (
    <>
      <span className="harness-ui-bound">≥ </span>
      {children}
    </>
  ) : (
    <>{children}</>
  )
}

/** Cache hit tone: a cold or broken prefix re-bills the whole prompt at the
    write premium every turn, which is worth a mark. */
const hitTone = (pct: number) => (pct >= 70 ? 'ok' : pct < 30 ? 'warn' : undefined)

interface UsageRowSpec {
  key: string
  label: string
  /** The counters a row reads; any of them partial makes the value `≥`. */
  reads: SummedKey[]
  value: (usage: UsageCounters) => ReactNode
  strong?: boolean
}

/** `turns` on the wire counts assistant messages — model steps, not the
    user's turns — so the label says steps. */
const USAGE_ROWS: UsageRowSpec[] = [
  { key: 'cost', label: 'Cost', strong: true, reads: ['cost_usd'], value: (u) => costOrDash(u.cost_usd) },
  { key: 'steps', label: 'Model steps', reads: [], value: (u) => num(u.turns) },
  {
    key: 'calls',
    label: 'Function calls',
    reads: [],
    value: (u) =>
      u.function_call_errors > 0
        ? `${num(u.function_calls)} · ${num(u.function_call_errors)} failed`
        : num(u.function_calls),
  },
  { key: 'in', label: 'Uncached input', reads: ['input_tokens'], value: (u) => tokensOrDash(u.input_tokens) },
  { key: 'out', label: 'Output', reads: ['output_tokens'], value: (u) => tokensOrDash(u.output_tokens) },
  {
    key: 'cache',
    label: 'Cache read · write',
    reads: ['cache_read_tokens', 'cache_write_tokens'],
    value: (u) =>
      u.cache_read_tokens == null && u.cache_write_tokens == null
        ? '—'
        : `${tokensOrDash(u.cache_read_tokens)} · ${tokensOrDash(u.cache_write_tokens)}`,
  },
  {
    key: 'hit',
    label: 'Cache hit',
    // A ratio, so never `≥`: a sum carries its own ratio over the sessions
    // that reported both counts (see `UsageTotals.cacheHit`).
    reads: [],
    value: (u) => {
      const pct = 'cacheHit' in u ? (u as UsageTotals).cacheHit : cacheHitPct(u)
      return pct == null ? (
        '—'
      ) : (
        <span className="harness-ui-hit" data-tone={hitTone(pct)}>
          {pct}%
        </span>
      )
    },
  },
]

function usageCell(row: UsageRowSpec, usage: UsageCounters, partial: readonly SummedKey[] = []) {
  return <Bound partial={row.reads.some((key) => partial.includes(key))}>{row.value(usage)}</Bound>
}

/** "a, b and 3 more" — a long list of ids would bury the sentence. */
function idList(ids: string[], self: string, limit = 3) {
  const names = ids.map((id) => (id === self ? 'this conversation' : id))
  const shown = names.slice(0, limit).join(', ')
  return names.length > limit ? `${shown} and ${names.length - limit} more` : shown
}

/**
 * This conversation's usage, and — only when it spawned sub-agents — the
 * same counters summed over its whole subtree in a second column.
 */
function UsageSection({
  state,
  tree,
  liveCost,
  onRetry,
}: {
  state: MetricsState
  tree: ConversationTree | null
  liveCost: number | null | undefined
  onRetry: () => void
}) {
  const headingId = useId()
  if (state.status === 'idle') return null
  const failure =
    state.status === 'error' ? (
      <StatusPanel
        variant="warn"
        role="status"
        icon={<TriangleAlert size={16} aria-hidden />}
        headline={tree ? "Couldn't refresh usage" : 'Usage unavailable'}
        detail={state.error}
        action={
          <Button variant="ghost" size="sm" onClick={onRetry}>
            Retry
          </Button>
        }
      />
    ) : null
  const costOnly =
    liveCost != null ? (
      <div className="harness-ui-usage-cost">
        <span>Cost</span>
        <span className="harness-ui-num">{formatCost(liveCost)}</span>
      </div>
    ) : null
  // No tree to show: still loading (the skeleton, below), failed, or the
  // answer does not carry this session (deleted mid-flight). The live cost
  // is all there is.
  if (!tree && state.status !== 'loading') {
    if (!failure && !costOnly) return null
    return (
      <section className="harness-ui-section" aria-labelledby={headingId}>
        <SectionHead id={headingId} title="Usage" />
        {costOnly}
        {failure}
      </section>
    )
  }
  if (tree && tree.self.turns === 0 && tree.subAgents.length === 0) {
    return costOnly ? (
      <section className="harness-ui-section" aria-labelledby={headingId}>
        <SectionHead id={headingId} title="Usage" />
        {costOnly}
      </section>
    ) : null
  }
  const shown = tree ? treeTotals(tree, liveCost) : null
  const self = shown?.self
  const total = shown?.total ?? null
  const subCount = tree?.subAgents.length ?? 0
  return (
    <section className="harness-ui-section" aria-labelledby={headingId} aria-busy={!tree}>
      <SectionHead id={headingId} title="Usage" />
      <TableViewport>
        <Table className="harness-ui-usage">
          {total ? (
            <TableHeader>
              <TableRow>
                <TableHead>
                  <span className="harness-ui-sr-only">Metric</span>
                </TableHead>
                <TableHead className="harness-ui-num">This conversation</TableHead>
                <TableHead className="harness-ui-num">
                  With {subCount} sub-agent{subCount === 1 ? '' : 's'}
                </TableHead>
              </TableRow>
            </TableHeader>
          ) : null}
          <TableBody>
            {USAGE_ROWS.map((row) => (
              <TableRow key={row.key} data-strong={row.strong ? '' : undefined}>
                <TableHead scope="row">{row.label}</TableHead>
                <TableCell className="harness-ui-num">
                  {self ? (
                    usageCell(row, self)
                  ) : row.key === 'cost' && liveCost != null ? (
                    formatCost(liveCost)
                  ) : (
                    <Skeleton className="harness-ui-skeleton" />
                  )}
                </TableCell>
                {total ? (
                  <TableCell className="harness-ui-num">{usageCell(row, total, total.partial)}</TableCell>
                ) : null}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableViewport>
      {total && total.partial.length > 0 && tree ? (
        <p className="harness-ui-note harness-ui-footnote">
          <Info size={16} aria-hidden />
          <span>
            ≥ leaves out what {total.unreported.length === 1 ? 'one session' : `${total.unreported.length} sessions`}{' '}
            did not report: {idList(total.unreported, tree.self.session_id)}.
          </span>
        </p>
      ) : null}
      {failure}
    </section>
  )
}

/** The steps / calls / failures / context / age line of one session. */
function sessionMeta(usage: SessionUsage, lead?: string) {
  const parts = lead ? [lead] : []
  parts.push(plural(usage.turns, 'step'))
  if (usage.function_call_errors > 0) parts.push(`${num(usage.function_call_errors)} failed`)
  const context = usage.context
  if (context && context.usable > 0) parts.push(`context ${percentOf(context.total, context.usable)}`)
  if (context?.timestamp) parts.push(ago(context.timestamp))
  return parts.join(' · ')
}

/** The same line for a group: its size (and how many a filter left), steps,
    failures, unpriced rows, age. Steps and failures are the whole group's. */
function groupMeta(group: SubAgentGroup) {
  const shown = group.rows.length - 1
  const t = group.totals
  const parts = [
    shown < group.size ? `${num(shown)} of ${plural(group.size, 'sub-agent')}` : plural(group.size, 'sub-agent'),
    plural(t.turns, 'step'),
  ]
  if (t.function_call_errors > 0) parts.push(`${num(t.function_call_errors)} failed`)
  const unpriced = t.unreported.length
  if (t.partial.includes('cost_usd') && unpriced) parts.push(`${num(unpriced)} without cost`)
  if (group.lastActive) parts.push(ago(group.lastActive))
  return parts.join(' · ')
}

function ago(ms: number) {
  const rel = formatRelative(ms)
  return rel === 'just now' || !rel ? 'just now' : `${rel} ago`
}

/** Share of the known total, as text and as a bar width. */
function share(cost: number | undefined, total: number | undefined) {
  if (cost == null || total == null || total <= 0) return null
  const pct = (cost / total) * 100
  return { label: pct > 0 && pct < 1 ? '<1%' : `${Math.round(pct)}%`, width: `${Math.max(pct, cost > 0 ? 1 : 0)}%` }
}

/** One ranked row: label, meta line, cost, share, and a bar of the share. */
function CostRow({
  label,
  mono = true,
  meta,
  cost,
  partial = false,
  total,
  expanded,
  onClick,
  action,
}: {
  label: string
  mono?: boolean
  meta: string
  cost: number | undefined
  partial?: boolean
  total: number | undefined
  expanded?: boolean
  onClick?: () => void
  /** The verb a screen reader hears before the label ("Open"); the row's
      own text — cost, share, steps — stays its accessible name. */
  action?: string
}) {
  const part = share(cost, total)
  const body = (
    <>
      <span className="harness-ui-cost-line">
        {expanded !== undefined ? (
          expanded ? (
            <ChevronDown className="harness-ui-sub-icon" size={16} aria-hidden />
          ) : (
            <ChevronRight className="harness-ui-sub-icon" size={16} aria-hidden />
          )
        ) : null}
        <span className="harness-ui-cost-main">
          <span className={mono ? 'harness-ui-sub-id' : 'harness-ui-cost-label'}>
            {action && onClick ? <span className="harness-ui-sr-only">{action} </span> : null}
            {label}
          </span>
          <span className="harness-ui-sub-meta">{meta}</span>
        </span>
        <span className="harness-ui-sub-cost">
          <Bound partial={partial}>{costOrDash(cost)}</Bound>
        </span>
        <span className="harness-ui-cost-share">{part?.label ?? ''}</span>
      </span>
      <span className="harness-ui-cost-bar" aria-hidden data-indent={expanded !== undefined ? '' : undefined}>
        <span style={{ width: part?.width ?? 0 }} />
      </span>
    </>
  )
  return onClick ? (
    <button
      type="button"
      className="harness-ui-cost-row"
      onClick={onClick}
      aria-expanded={expanded}
    >
      {body}
    </button>
  ) : (
    <div className="harness-ui-cost-row">{body}</div>
  )
}

/** How many of the biggest direct sub-agents the overview ranks. */
const RANKED = 5

/**
 * Where the subtree's cost went: this conversation, then its direct
 * sub-agents (each with everything it spawned) by cost, the rest folded into
 * one row. The full list is the sub-agents page; navigating between
 * conversations is the chat side panel's job.
 */
function CostSection({
  tree,
  liveCost,
  onOpenSession,
  onShowAll,
  restoreFocus,
}: {
  tree: ConversationTree
  liveCost: number | null | undefined
  onOpenSession?: (sessionId: string) => void
  onShowAll: (expand?: string) => void
  restoreFocus: boolean
}) {
  const headingId = useId()
  const allButton = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    if (restoreFocus) allButton.current?.focus()
  }, [restoreFocus])
  if (tree.subAgents.length === 0) return null
  const { self, total } = treeTotals(tree, liveCost)
  const totalCost = total?.cost_usd
  const groups = sortGroups(subAgentGroups(tree), 'cost')
  // Folding a single leftover into "1 more" hides it for nothing.
  const cut = groups.length > RANKED + 1 ? RANKED : groups.length
  const ranked = groups.slice(0, cut)
  const rest = groups.slice(cut)
  const nested = groups.some((group) => group.rows.length > 1)
  const restTotals = rest.length ? sumUsage(rest.flatMap((group) => group.rows)) : null
  const count = tree.subAgents.length
  return (
    <section className="harness-ui-section" aria-labelledby={headingId}>
      <SectionHead
        id={headingId}
        title="Where the cost went"
        meta={nested ? `${plural(count, 'sub-agent')} · ${num(groups.length)} direct` : plural(count, 'sub-agent')}
      />
      <div className="harness-ui-cost-list">
        <CostRow
          label="This conversation"
          mono={false}
          meta={[
            plural(self.turns, 'step'),
            plural(self.function_calls, 'call'),
            self.function_call_errors ? `${num(self.function_call_errors)} failed` : null,
          ]
            .filter(Boolean)
            .join(' · ')}
          cost={self.cost_usd}
          total={totalCost}
        />
        {ranked.map((group) => (
          <CostRow
            key={group.head.session_id}
            label={group.head.session_id}
            meta={group.rows.length > 1 ? groupMeta(group) : sessionMeta(group.head)}
            cost={group.totals.cost_usd}
            partial={group.totals.partial.includes('cost_usd')}
            total={totalCost}
            action={group.rows.length > 1 ? 'Show the sub-agents of' : 'Open'}
            onClick={
              group.rows.length > 1
                ? () => onShowAll(group.head.session_id)
                : onOpenSession
                  ? () => onOpenSession(group.head.session_id)
                  : undefined
            }
          />
        ))}
        {restTotals ? (
          <CostRow
            label={`${rest.length} more`}
            mono={false}
            meta={[
              plural(
                rest.reduce((n, g) => n + g.rows.length, 0),
                'sub-agent',
              ),
              plural(restTotals.turns, 'step'),
              restTotals.function_call_errors ? `${num(restTotals.function_call_errors)} failed` : null,
            ]
              .filter(Boolean)
              .join(' · ')}
            cost={restTotals.cost_usd}
            partial={restTotals.partial.includes('cost_usd')}
            total={totalCost}
            action="Show"
            onClick={() => onShowAll()}
          />
        ) : null}
      </div>
      {nested || rest.length ? (
        <Button ref={allButton} variant="ghost" size="sm" className="harness-ui-all" onClick={() => onShowAll()}>
          All {plural(count, 'sub-agent')}
          <ChevronRight size={16} aria-hidden />
        </Button>
      ) : null}
    </section>
  )
}

const SORTS: { value: GroupSort; label: string; icon: false }[] = [
  { value: 'cost', label: 'Cost', icon: false },
  { value: 'recent', label: 'Recent', icon: false },
  { value: 'failed', label: 'Failed', icon: false },
]

/** Rows inside a group: the head first, the rest by the page's sort. */
function sortRows(rows: SessionUsage[], by: GroupSort) {
  const [head, ...rest] = rows
  const cost = (row: SessionUsage) => row.cost_usd ?? 0
  rest.sort((a, b) =>
    by === 'recent'
      ? (b.context?.timestamp ?? 0) - (a.context?.timestamp ?? 0) || cost(b) - cost(a)
      : by === 'failed'
        ? b.function_call_errors - a.function_call_errors || cost(b) - cost(a)
        : cost(b) - cost(a),
  )
  return [head, ...rest]
}

/**
 * Every sub-agent, grouped under the conversation's direct sub-agents: a
 * page inside the same dialog or sheet, with a filter by id and three sorts.
 * A group opens and closes in place; a row opens that conversation.
 */
function SubAgentsPage({
  tree,
  liveCost,
  initialExpanded,
  onBack,
  onOpenSession,
}: {
  tree: ConversationTree
  liveCost: number | null | undefined
  initialExpanded?: string
  onBack: () => void
  onOpenSession?: (sessionId: string) => void
}) {
  const back = useRef<HTMLButtonElement>(null)
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<GroupSort>('cost')
  const [expanded, setExpanded] = useState<Set<string>>(
    () => new Set(initialExpanded ? [initialExpanded] : []),
  )
  useEffect(() => {
    back.current?.focus()
  }, [])
  const all = subAgentGroups(tree)
  const subTotals = sumUsage(tree.subAgents.map((row) => row.usage))
  const totalCost = treeTotals(tree, liveCost).total?.cost_usd
  const groups = filterGroups(sortGroups(all, sort), query)
  const needle = query.trim().toLowerCase()
  const filtering = needle !== ''
  const matches = filtering
    ? tree.subAgents.filter((row) => row.usage.session_id.toLowerCase().includes(needle)).length
    : 0
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  useEffect(() => {
    if (!needle) return
    setAnnouncement((prev) => ({
      seq: (prev?.seq ?? 0) + 1,
      text: `${plural(matches, 'sub-agent')} match`,
      urgency: 'polite',
    }))
  }, [needle, matches])
  const toggle = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const summary = [
    num(tree.subAgents.length),
    subTotals.cost_usd != null
      ? `${subTotals.partial.includes('cost_usd') ? '≥ ' : ''}${formatCost(subTotals.cost_usd)}`
      : null,
    plural(subTotals.turns, 'step'),
    subTotals.function_call_errors ? `${num(subTotals.function_call_errors)} failed` : null,
  ]
    .filter(Boolean)
    .join(' · ')
  return (
    <div className="harness-ui-pop" data-page="sub-agents">
      <div className="harness-ui-pop-head harness-ui-page-head">
        <IconButton ref={back} label="Back to context" tooltip={false} onClick={onBack}>
          <ArrowLeft size={16} aria-hidden />
        </IconButton>
        <div className="harness-ui-page-title">
          <DialogTitle>Sub-agents</DialogTitle>
          <DialogDescription className="harness-ui-pop-usage">{summary}</DialogDescription>
        </div>
      </div>
      <div className="harness-ui-page-tools">
        <SearchField
          value={query}
          onChange={setQuery}
          aria-label="Filter sub-agents by id"
          placeholder="Filter by id"
          className="harness-ui-page-search"
        />
        <SegmentedControl variant="radio" aria-label="Sort by" value={sort} onChange={setSort} options={SORTS} />
      </div>
      {filtering && matches > 0 ? (
        <p className="harness-ui-note">
          {num(matches)} of {plural(tree.subAgents.length, 'sub-agent')} match
        </p>
      ) : null}
      <LiveRegion announcement={announcement} />
      <div className="harness-ui-cost-list">
        {groups.map((group) => {
          const nested = group.rows.length > 1
          const open = nested && (filtering || expanded.has(group.head.session_id))
          const id = group.head.session_id
          return (
            <div key={id} className="harness-ui-group">
              <CostRow
                label={id}
                meta={nested ? groupMeta(group) : sessionMeta(group.head)}
                cost={group.totals.cost_usd}
                partial={group.totals.partial.includes('cost_usd')}
                total={totalCost}
                expanded={nested ? open : undefined}
                action={nested ? undefined : 'Open'}
                // While filtering every matching group is open; a toggle
                // there would do nothing visible.
                onClick={
                  nested
                    ? filtering
                      ? undefined
                      : () => toggle(id)
                    : onOpenSession
                      ? () => onOpenSession(id)
                      : undefined
                }
              />
              {open ? (
                <div className="harness-ui-group-rows">
                  {sortRows(group.rows, sort).map((row, index) => (
                    <ListItem
                      key={row.session_id}
                      as={onOpenSession ? 'button' : 'div'}
                      onClick={onOpenSession ? () => onOpenSession(row.session_id) : undefined}
                      leading={<CornerDownRight size={16} aria-hidden />}
                      label={<span className="harness-ui-sub-id">{row.session_id}</span>}
                      description={sessionMeta(row, index === 0 ? 'spawned the rest' : undefined)}
                      trailing={
                        <span className="harness-ui-sub-trailing">
                          <span className="harness-ui-sub-cost" data-unpriced={row.cost_usd == null && row.turns > 0 ? '' : undefined}>
                            {row.cost_usd == null ? (row.turns > 0 ? 'no cost' : '—') : formatCost(row.cost_usd)}
                          </span>
                          {onOpenSession ? (
                            <ChevronRight className="harness-ui-sub-icon" size={16} aria-hidden />
                          ) : null}
                        </span>
                      }
                    />
                  ))}
                </div>
              ) : null}
            </div>
          )
        })}
        {filtering && groups.length === 0 ? (
          <p className="harness-ui-note">No sub-agent id contains “{query.trim()}”.</p>
        ) : null}
      </div>
    </div>
  )
}

const MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent)

/**
 * The session id (the `state::get` / session-store key) in a read-only field
 * with a copy button. A failed copy — no clipboard access — selects the id so
 * the keyboard shortcut finishes the job, and says so until a copy works.
 */
function SessionIdRow({ sessionId }: { sessionId: string }) {
  const inputId = useId()
  const input = useRef<HTMLInputElement>(null)
  const { state, copy } = useCopyFlash(sessionId, 1400)
  const [failed, setFailed] = useState(false)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  useEffect(() => {
    if (state === 'idle') return
    if (state === 'failed') {
      setFailed(true)
      input.current?.focus()
      input.current?.select()
    } else {
      setFailed(false)
    }
    setAnnouncement((prev) => ({
      seq: (prev?.seq ?? 0) + 1,
      text:
        state === 'copied'
          ? 'Session id copied'
          : `Couldn't copy. The session id is selected, press ${MAC ? 'Command' : 'Control'} C.`,
      urgency: state === 'copied' ? 'polite' : 'assertive',
    }))
  }, [state])
  const copied = state === 'copied'
  return (
    <div className="harness-ui-field-block">
      <div className="harness-ui-field-row">
        <label htmlFor={inputId} className="harness-ui-field-label">
          Session
        </label>
        <div className="harness-ui-field">
          {/* Out of the tab order: the dialog focuses its first tabbable on
              open, and a focused, fully selected id reads as an edit. The
              copy button is the keyboard path; a failed copy focuses this
              field itself. */}
          <input
            id={inputId}
            ref={input}
            className="harness-ui-field-input"
            readOnly
            tabIndex={-1}
            value={sessionId}
          />
          <IconButton
            label={copied ? 'Copied' : 'Copy session id'}
            onClick={copy}
            // The dialog may focus this button on open; a tooltip that pops
            // up on that programmatic focus is noise. The label stays.
            tooltip={false}
            data-copy={state}
            className="harness-ui-copy"
          >
            {copied ? <Check size={16} aria-hidden /> : <Copy size={16} aria-hidden />}
          </IconButton>
        </div>
      </div>
      {failed ? (
        <p className="harness-ui-copy-failed">
          Couldn't copy. The id is selected — press{' '}
          <KeyCombo binding="Mod+C" platform={MAC ? 'mac' : 'other'} />.
        </p>
      ) : null}
      <LiveRegion announcement={announcement} />
    </div>
  )
}

function ParentRow({
  parentId,
  onOpenSession,
}: {
  parentId: string
  onOpenSession?: (sessionId: string) => void
}) {
  return (
    <div className="harness-ui-field-row">
      <span className="harness-ui-field-label">Spawned by</span>
      {onOpenSession ? (
        <button
          type="button"
          className="harness-ui-parent"
          aria-label={`Open parent conversation ${parentId}`}
          onClick={() => onOpenSession(parentId)}
        >
          <span className="harness-ui-parent-id">{parentId}</span>
          <ArrowUpRight size={16} aria-hidden />
        </button>
      ) : (
        <span className="harness-ui-parent">
          <span className="harness-ui-parent-id">{parentId}</span>
        </span>
      )}
    </div>
  )
}

/** The dialog's title and one-line descriptor: model, then usage. */
function PopoverHead({ model, usage }: { model: string; usage: string }) {
  return (
    <div className="harness-ui-pop-head">
      <DialogTitle>Context</DialogTitle>
      <DialogDescription className="harness-ui-pop-usage">
        {model} · {usage}
      </DialogDescription>
    </div>
  )
}

interface ContextPopoverProps {
  snapshot: ContextSnapshot | null
  modelId?: string
  contextWindow?: number
  sessionId: string
  metrics: MetricsState
  onRetry: () => void
  onOpenSession?: (sessionId: string) => void
}

/** Which page of the breakdown is showing; the sub-agents page may open on
    one group. Closing the dialog unmounts it, so it always reopens on the
    overview. */
type PopoverView = { page: 'overview'; returned?: boolean } | { page: 'sub-agents'; expand?: string }

function ContextPopover({
  snapshot,
  modelId,
  contextWindow,
  sessionId,
  metrics,
  onRetry,
  onOpenSession,
}: ContextPopoverProps) {
  const [view, setView] = useState<PopoverView>({ page: 'overview' })
  const root = useRef<HTMLDivElement>(null)
  // A new page starts at its top: the dialog content (desktop) or the sheet
  // body (phone) is the scroller. A layout effect, so it runs before the
  // overview's effect focuses (and scrolls to) the "All sub-agents" button.
  useLayoutEffect(() => {
    const el = root.current
    if (!el) return
    el.scrollTop = 0
    const dialog = el.closest('[role="dialog"]')
    if (dialog) dialog.scrollTop = 0
  }, [view.page])
  const data = metrics.status === 'idle' ? undefined : metrics.data
  const tree = data ? conversationTree(data, sessionId) : null
  const live = snapshot && snapshot.usable > 0 ? snapshot : null
  const liveCost = live?.session_cost_usd
  if (view.page === 'sub-agents' && tree) {
    return (
      <div ref={root} className="harness-ui-pop-root">
        <SubAgentsPage
          tree={tree}
          liveCost={liveCost}
          initialExpanded={view.expand}
          onBack={() => setView({ page: 'overview', returned: true })}
          onOpenSession={onOpenSession}
        />
      </div>
    )
  }
  const parentId = tree?.self.parent_session_id
  let usage: string
  if (live) {
    const pct = Math.round(Math.min(1, live.total / live.usable) * 100)
    const approx = !live.estimator || live.estimator === 'heuristic' ? '~' : ''
    usage = `${approx}${formatTokens(live.total)} of ${formatTokens(live.usable)} · ${pct}%`
  } else {
    usage = 'waiting for the first step'
  }
  return (
    <div ref={root} className="harness-ui-pop">
      <PopoverHead model={live?.model || modelId || 'model'} usage={usage} />
      {live ? (
        <WindowSection snapshot={live} />
      ) : (
        <section className="harness-ui-section" aria-label="Context window">
          {contextWindow && contextWindow > 0 ? (
            <>
              <div className="harness-ui-stack" />
              <div className="harness-ui-legend">
                <LegendRow
                  row={{ key: 'free', label: 'Free', color: COLOR_FREE, tokens: contextWindow }}
                  usable={contextWindow}
                />
              </div>
            </>
          ) : null}
          <p className="harness-ui-note">
            {contextWindow && contextWindow > 0
              ? 'The breakdown appears after the first generation step.'
              : 'The selected model did not report a context-window limit.'}
          </p>
        </section>
      )}
      <UsageSection state={metrics} tree={tree} liveCost={liveCost} onRetry={onRetry} />
      {tree ? (
        <CostSection
          tree={tree}
          liveCost={liveCost}
          onOpenSession={onOpenSession}
          onShowAll={(expand) => setView({ page: 'sub-agents', expand })}
          restoreFocus={view.page === 'overview' && view.returned === true}
        />
      ) : null}
      <div className="harness-ui-pop-foot">
        <SessionIdRow sessionId={sessionId} />
        {parentId ? <ParentRow parentId={parentId} onOpenSession={onOpenSession} /> : null}
      </div>
    </div>
  )
}

interface ContextChipSurfaceProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  triggerLabel: string
  trigger: ReactNode
  popover: ReactNode
  /** The trigger carries a `used/usable` count — what a narrow pane keeps
      when it hides the bar and the eyebrow (see styles.css). */
  hasCounts?: boolean
}

/**
 * One surface for every context-chip state: the chip is the trigger, the
 * breakdown opens as a `Dialog` on desktop and a `BottomSheet` on phones.
 * Focus, Escape, outside click and scroll locking are the dialog's.
 */
function ContextChipSurface({
  open,
  onOpenChange,
  triggerLabel,
  trigger,
  popover,
  hasCounts = false,
}: ContextChipSurfaceProps) {
  const phone = usePhone()
  const Root = phone ? BottomSheet : Dialog
  const Trigger = phone ? BottomSheetTrigger : DialogTrigger
  return (
    <div className="harness-ui-chip" data-has-counts={hasCounts ? '' : undefined}>
      <Root open={open} onOpenChange={onOpenChange}>
        <Trigger asChild>
          <button type="button" className="harness-ui-chip-btn" aria-label={triggerLabel}>
            {trigger}
          </button>
        </Trigger>
        {phone ? (
          <BottomSheetContent className="harness-ui-sheet">{popover}</BottomSheetContent>
        ) : (
          <DialogContent className="harness-ui-dialog">{popover}</DialogContent>
        )}
      </Root>
    </div>
  )
}

export function createContextChip(host: Host) {
  return function ContextChip({
    sessionId,
    modelId,
    contextWindow,
  }: SessionChipProps) {
    const [snapshot, setSnapshot] = useState<ContextSnapshot | null>(null)
    const [open, setOpen] = useState(false)
    const metrics = useConversationMetrics(host, sessionId, open, snapshot?.turn_id)
    const selectConversation = host.chat?.selectConversation
    const onOpenSession = selectConversation
      ? (id: string) => {
          setOpen(false)
          selectConversation(id)
        }
      : undefined

    // Both the hydration read and the streamed trigger write this state;
    // keep whichever snapshot is newest so a slow state::get can never
    // overwrite a fresher streamed step.
    const acceptNewer = (value: ContextSnapshot) =>
      setSnapshot((current) =>
        current && current.timestamp >= value.timestamp ? current : value,
      )

    useEffect(() => {
      let cancelled = false
      setSnapshot(null)
      setOpen(false)
      host.iii
        .trigger('state::get', { scope: 'harness_context', key: sessionId })
        .then((value) => {
          if (cancelled) return
          if (isSnapshot(value) && value.session_id === sessionId)
            acceptNewer(value)
        })
        .catch(() => {})
      return () => {
        cancelled = true
      }
    }, [host, sessionId])

    // Snapshots are written after every generate step; the state worker's
    // `state` trigger streams each write (engine-side scope/key filter), so
    // long multi-step turns tick live without any polling.
    useEffect(() => {
      // The id carries the session, because the engine-side filter is keyed to
      // one: two chips mounted at once (two sessions visible, or an unmount
      // racing the next mount) would otherwise register conflicting filters
      // under one id, and either teardown would take the other's stream with
      // it. `on()` appends `::<browserId>` itself, so the trigger repeats it
      // to address the same handler.
      const eventFn = `${STATE_FN}::${sessionId}`
      const offHandler = host.iii.on<StateEvent>(eventFn, (event) => {
        if (!event || event.key !== sessionId) return
        if (event.event_type === 'state:deleted') return
        if (
          isSnapshot(event.new_value) &&
          event.new_value.session_id === sessionId
        )
          acceptNewer(event.new_value)
      })
      const offTrigger = host.iii.registerTrigger({
        type: 'state',
        function_id: `${eventFn}::${host.iii.browserId}`,
        config: { scope: 'harness_context', key: sessionId },
      })
      return () => {
        offTrigger()
        offHandler()
      }
    }, [host, sessionId])

    const caret = <ChevronDown className="harness-ui-chip-caret" size={16} aria-hidden />
    const label = (
      <span className={`${uiClasses.eyebrow} harness-ui-chip-label`}>ctx</span>
    )
    const popover = (
      <ContextPopover
        snapshot={snapshot}
        modelId={modelId}
        contextWindow={contextWindow}
        sessionId={sessionId}
        metrics={metrics.state}
        onRetry={metrics.retry}
        onOpenSession={onOpenSession}
      />
    )

    if (!snapshot || snapshot.usable <= 0) {
      if (contextWindow && contextWindow > 0) {
        return (
          <ContextChipSurface
            open={open}
            onOpenChange={setOpen}
            triggerLabel={`context: waiting for usage, ${contextWindow.toLocaleString()} token window — click for the breakdown`}
            hasCounts
            trigger={
              <>
                {label}
                <span
                  className="harness-ui-chip-bar"
                  role="progressbar"
                  aria-label="context window usage"
                  aria-valuenow={0}
                  aria-valuemin={0}
                  aria-valuemax={100}
                >
                  <span className="harness-ui-chip-fill" style={{ width: 0 }} />
                </span>
                <span className="harness-ui-chip-counts">
                  0/{formatTokens(contextWindow)}
                </span>
                {caret}
              </>
            }
            popover={popover}
          />
        )
      }
      return (
        <ContextChipSurface
          open={open}
          onOpenChange={setOpen}
          triggerLabel="context: waiting for usage — click for the breakdown"
          trigger={
            <>
              {label}
              <span className="harness-ui-chip-empty">—</span>
              {caret}
            </>
          }
          popover={popover}
        />
      )
    }

    const ratio = Math.min(1, snapshot.total / snapshot.usable)
    const pct = Math.round(ratio * 100)
    const tone = toneFor(ratio)

    return (
      <ContextChipSurface
        open={open}
        onOpenChange={setOpen}
        triggerLabel={`context: ${snapshot.total.toLocaleString()} of ${snapshot.usable.toLocaleString()} tokens (${pct}%) — click for the breakdown`}
        hasCounts
        trigger={
          <>
            {label}
            <span
              className="harness-ui-chip-bar"
              role="progressbar"
              aria-label="context window usage"
              aria-valuenow={pct}
              aria-valuemin={0}
              aria-valuemax={100}
            >
              <span
                className="harness-ui-chip-fill"
                style={{ width: `${pct}%`, background: TONE_COLOR[tone] }}
              />
            </span>
            {/* The bar, the percentage and the counts were three encodings of
                one quantity. The bar carries proportion; the counts carry the
                number you actually act on. The percentage keeps its job in the
                tooltip and the popover, and hands its tone to the counts. */}
            <span
              className="harness-ui-chip-counts"
              style={tone === 'ok' ? undefined : { color: TONE_COLOR[tone] }}
            >
              {formatTokens(snapshot.total)}/{formatTokens(snapshot.usable)}
            </span>
            {caret}
          </>
        }
        popover={popover}
      />
    )
  }
}
