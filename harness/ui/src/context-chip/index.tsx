/**
 * The `context` session chip — a live view of the session's context window
 * matching the console's ContextUsage meter (a ring gauge, `12.3k / 200k`
 * counts, a caret). Hydrates from the stored snapshot (`state::get` on
 * `harness_context/<session id>`) on mount and per session change; stays live
 * over the state worker's own `state` trigger for that key (Message-path
 * binding, GC'd with the tab), which fires on every generate step. Click opens
 * the breakdown — a `Dialog` on desktop, a `BottomSheet` on phones — led by a
 * hero gauge (share used, used / free / window), then what the window is made
 * of (stacked bar, iconed legend, the last step as tiles), the conversation's
 * usage from `harness::metrics` (plus a with-sub-agents column when it
 * spawned any), where that subtree's cost went (with a page listing every
 * sub-agent), and the session id with copy.
 *
 * The chip answers to the PANE, not the viewport: under a 30rem-wide chat
 * pane (a phone, or a narrow split) the caret hides (styles.css,
 * `@container`); the ring and the counts stay, since they are the part you
 * act on.
 */

import {
  Badge,
  BottomSheet,
  BottomSheetContent,
  BottomSheetTrigger,
  Button,
  Chip,
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
import { errorMessage, formatRelative, unwrapEnvelope } from '@iii-dev/console-ui/format'
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
import { compactions, growth, type StepPoint, stepPoints } from '../lib/history'
import { RingGauge } from './icons'
import { categories, type LegendEntry, legendRows, percentOf } from '../lib/legend'
import {
  type ContextSnapshot,
  type ConversationTree,
  cacheHitPct,
  conversationTree,
  filterGroups,
  type GroupSort,
  isSnapshot,
  type MetricsResponse,
  type ModelTotals,
  modelLabel,
  modelTotals,
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
import { TONE_COLOR, type Tone, toneFor } from '../lib/tone'

/** Per-tab handler id (host.iii.on namespaces it `::<browserId>`). */
const STATE_FN = 'iii::harness-ui::ctx-state'
const METRICS_FN = 'harness::metrics'
const MESSAGES_FN = 'session::messages'
/** One page of assistant steps; a long session pages. */
const HISTORY_PAGE = 500
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
      // Only this conversation's subtree (the modal never shows its parent
      // or siblings) and no trace scan: on a 179-session root both cost
      // more than everything else. A harness without these options ignores
      // them, returns the whole tree, and `conversationTree` cuts it.
      .trigger(
        METRICS_FN,
        { session_id: sessionId, subtree: true, omit_traces: true },
        { timeoutMs: METRICS_TIMEOUT_MS },
      )
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

type HistoryState =
  | { status: 'idle' }
  | { status: 'loading'; points?: StepPoint[] }
  | { status: 'ready'; points: StepPoint[]; key: string }
  | { status: 'error'; points?: StepPoint[] }

/**
 * The window over the conversation's steps, from the session's assistant
 * messages (usage projection: no content, just what each step billed).
 * Read when the breakdown opens on a turn it has no series for yet, and
 * again when a new turn lands while it stays open. A read that fails keeps
 * the last series; the chart is a convenience, never a blocker.
 */
function useContextHistory(host: Host, sessionId: string, open: boolean, turnId: string | undefined) {
  const [state, setState] = useState<HistoryState>({ status: 'idle' })
  const current = useRef(state)
  current.current = state
  const key = `${sessionId}\n${turnId ?? ''}`
  useEffect(() => {
    setState({ status: 'idle' })
  }, [sessionId])
  useEffect(() => {
    if (!open) return
    const known = current.current
    if (known.status === 'ready' && known.key === key) return
    let cancelled = false
    setState((prev) => ({ status: 'loading', points: prev.status === 'idle' ? undefined : prev.points }))
    const pages: unknown[] = []
    const read = async () => {
      let cursor: string | undefined
      for (let page = 0; page < 40; page++) {
        const payload: Record<string, unknown> = {
          session_id: sessionId,
          roles: ['assistant'],
          content: 'usage',
          include_custom: false,
          limit: HISTORY_PAGE,
        }
        if (cursor) payload.cursor = cursor
        const value = await host.iii.trigger(MESSAGES_FN, payload, { timeoutMs: 30_000 })
        pages.push(value)
        const next = (unwrapEnvelope(value) as { next_cursor?: unknown } | null)?.next_cursor
        if (typeof next !== 'string' || next === '' || next === cursor) break
        cursor = next
      }
      return stepPoints(pages)
    }
    read()
      .then((points) => {
        if (!cancelled) setState({ status: 'ready', points, key })
      })
      .catch(() => {
        if (!cancelled) setState((prev) => ({ status: 'error', points: prev.status === 'idle' ? undefined : prev.points }))
      })
    return () => {
      cancelled = true
    }
  }, [host, sessionId, open, key])
  return state
}

/** Hours and minutes, the user's locale; the day is not worth the width. */
const clock = (ms: number) =>
  ms > 0 ? new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' }) : ''

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

/** "a, b and 3 more" — a long list of ids would bury the sentence. */
function idList(ids: string[], self: string, limit = 3) {
  const names = ids.map((id) => (id === self ? 'this conversation' : id))
  const shown = names.slice(0, limit).join(', ')
  return names.length > limit ? `${shown} and ${names.length - limit} more` : shown
}

/** The headline: how much of the window is in use, and the share as a pill. */
function Hero({ total, usable, estimated }: { total: number; usable: number; estimated: boolean }) {
  const ratio = usable > 0 ? Math.min(1, total / usable) : 0
  const pct = Math.round(ratio * 100)
  return (
    <div className="harness-ui-hero">
      <div className="harness-ui-hero-main">
        <span className="harness-ui-hero-num">{`${estimated ? '~' : ''}${formatTokens(total)}`}</span>
        <span className="harness-ui-hero-of">of {formatTokens(usable)} tokens</span>
      </div>
      <span className="harness-ui-pill" data-tone={toneFor(ratio)}>
        {`${pct}%`}
      </span>
    </div>
  )
}

/**
 * Before the first reply there is nothing to break down. Instead of a zero
 * hero over an empty track, a quiet placeholder: a ghost of the bar to
 * come (three faint segments, breathing while the first step runs) and
 * one sentence saying what will fill it and when.
 */
function EmptyWindow({ contextWindow }: { contextWindow?: number }) {
  const sized = contextWindow !== undefined && contextWindow > 0
  return (
    <section className="harness-ui-section" aria-label="Context window">
      <div className="harness-ui-empty">
        <div className={`harness-ui-empty-track ${uiClasses.pulse}`} aria-hidden>
          <span style={{ flexBasis: '38%' }} />
          <span style={{ flexBasis: '22%' }} />
          <span style={{ flexBasis: '12%' }} />
        </div>
        <p className="harness-ui-empty-title">
          {sized ? `${formatTokens(contextWindow)} token window` : 'Nothing counted yet'}
        </p>
        <p className="harness-ui-empty-text">
          {sized
            ? 'Nothing counted yet. The breakdown of what the window holds appears after the first reply.'
            : 'The selected model did not report a context-window limit, so usage shows as a count without a share.'}
        </p>
      </div>
    </section>
  )
}

/** The window as a thin bar: one rounded segment per category, 2px apart. */
function StackBar({ pct, children }: { pct: number; children?: ReactNode }) {
  return (
    <div
      className="harness-ui-stack"
      role="progressbar"
      aria-label="context window usage"
      aria-valuenow={pct}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      {children}
    </div>
  )
}

/** One legend row: swatch, label, tokens, share. */
function LegendRow({ row, usable }: { row: LegendEntry; usable: number }) {
  return (
    <div className="harness-ui-legend-row" data-group={row.group ? '' : undefined}>
      <span
        className="harness-ui-swatch"
        style={row.color ? { background: row.color } : { visibility: 'hidden' }}
      />
      <span className="harness-ui-legend-text">{row.label}</span>
      {row.badge ? <Badge variant="warn">{row.badge}</Badge> : null}
      <span className="harness-ui-legend-val">{formatTokens(row.tokens)}</span>
      <span className="harness-ui-legend-pct">{percentOf(row.tokens, usable)}</span>
    </div>
  )
}

/** How many recent steps the bars show. */
const BARS = 24

/**
 * The window per model step, as bars for the recent steps: a drop is a
 * compaction. The eyebrow's right side reads the pace and the steps left
 * at that pace, or the hovered step. Nothing until two steps exist.
 */
function StepBars({
  history,
  usable,
  current,
}: {
  history: HistoryState
  usable: number
  current: number
}) {
  const [hover, setHover] = useState<number | null>(null)
  const points = history.status === 'idle' ? undefined : history.points
  if (!points) {
    return history.status === 'loading' ? (
      <div className="harness-ui-steps">
        <div className="harness-ui-steps-head">
          <span className={uiClasses.eyebrow}>Per step</span>
        </div>
        <Skeleton className="harness-ui-bars-skeleton" />
      </div>
    ) : null
  }
  if (points.length < 2) return null
  const drops = new Set(compactions(points))
  const pace = growth(points, usable, current)
  const first = Math.max(0, points.length - BARS)
  const shown = points.slice(first)
  const peak = Math.max(1, ...shown.map((p) => p.prompt))
  const picked = hover != null ? points[hover] : undefined
  let meta: ReactNode
  if (picked) {
    meta = (
      <>
        Step {num(picked.step)} · <span className="harness-ui-strong">{formatTokens(picked.prompt)}</span>
        {picked.cost != null ? <> · {formatCost(picked.cost)}</> : null}
        {drops.has(hover as number) ? ' · compacted' : null}
        {picked.at ? <> · {clock(picked.at)}</> : null}
      </>
    )
  } else if (pace && pace.perStep > 0) {
    meta = (
      <>
        Avg <span className="harness-ui-strong">+{formatTokens(pace.perStep)}</span> / step
        {pace.stepsLeft != null ? (
          <> · {pace.stepsLeft === 0 ? 'full next step' : `~${num(pace.stepsLeft)} left`}</>
        ) : null}
      </>
    )
  } else {
    meta = <>{plural(points.length, 'step')} · not growing</>
  }
  return (
    <div className="harness-ui-steps">
      <div className="harness-ui-steps-head">
        <span className={uiClasses.eyebrow}>Per step</span>
        <span className="harness-ui-steps-meta">{meta}</span>
      </div>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: a hover layer; the readout is text above */}
      <div className="harness-ui-bars" onPointerLeave={() => setHover(null)}>
        {shown.map((point, i) => {
          const index = first + i
          return (
            <div
              key={point.step}
              className="harness-ui-bar"
              data-hover={hover === index ? '' : undefined}
              data-compacted={drops.has(index) ? '' : undefined}
              title={`Step ${num(point.step)} · ${formatTokens(point.prompt)}`}
              onPointerEnter={() => setHover(index)}
            >
              <span style={{ height: `${Math.max(6, (point.prompt / peak) * 100)}%` }} />
            </div>
          )
        })}
      </div>
    </div>
  )
}

/** A labelled number with a thin bar under it; `dashed` for an uncapped one. */
function Tile({
  label,
  value,
  of,
  ratio,
  dashed = false,
  tone,
}: {
  label: string
  value: ReactNode
  of?: ReactNode
  ratio?: number
  dashed?: boolean
  tone?: 'ok' | 'warn'
}) {
  return (
    <div className="harness-ui-tile">
      <span className="harness-ui-tile-label">
        {label}
        {tone ? <span className="harness-ui-tile-dot" data-tone={tone} /> : null}
      </span>
      <span className="harness-ui-tile-val">
        <span className="harness-ui-strong">{value}</span>
        {of ? <span className="harness-ui-tile-of"> {of}</span> : null}
      </span>
      <span className="harness-ui-tile-bar" data-dashed={dashed ? '' : undefined}>
        {ratio != null ? <span style={{ width: `${Math.round(Math.min(1, Math.max(0, ratio)) * 100)}%` }} /> : null}
      </span>
    </div>
  )
}

/**
 * The three numbers under the chart: what is left, how well the cache is
 * working, and what the conversation has cost (with its subtree, when it
 * spawned sub-agents). Cost comes from `harness::metrics`; until it answers
 * the live snapshot's running cost stands in.
 */
function StatTiles({
  snapshot,
  state,
  tree,
  liveCost,
}: {
  snapshot: ContextSnapshot
  state: MetricsState
  tree: ConversationTree | null
  liveCost: number | null | undefined
}) {
  const usable = snapshot.usable
  const free = Math.max(0, snapshot.free)
  const shown = tree ? treeTotals(tree, liveCost) : null
  const self = shown?.self
  const total = shown?.total ?? null
  const hit: number | null =
    self && self.cacheHit != null
      ? self.cacheHit
      : snapshot.usage && (snapshot.usage.cache_read ?? 0) + (snapshot.usage.cache_write ?? 0) > 0
        ? cacheHitPct({
            turns: 1,
            function_calls: 0,
            function_call_errors: 0,
            input_tokens: snapshot.usage.input ?? 0,
            cache_read_tokens: snapshot.usage.cache_read ?? 0,
            cache_write_tokens: snapshot.usage.cache_write,
          })
        : null
  const cost = self ? self.cost_usd : liveCost ?? undefined
  const loading = !self && state.status === 'loading' && cost == null
  return (
    <div className="harness-ui-tiles">
      <Tile label="Free" value={formatTokens(free)} of={`of ${formatTokens(usable)}`} ratio={usable > 0 ? free / usable : 0} />
      <Tile
        label="Cache hit"
        value={hit == null ? '—' : `${hit}%`}
        of={hit == null ? undefined : self ? 'of prompts' : 'last step'}
        ratio={hit == null ? undefined : hit / 100}
        tone={hit == null ? undefined : hitTone(hit)}
      />
      <Tile
        label="Cost"
        value={
          loading ? (
            <Skeleton className="harness-ui-skeleton" />
          ) : (
            <Bound partial={self?.partial.includes('cost_usd') ?? false}>{costOrDash(cost)}</Bound>
          )
        }
        of={
          total && total.cost_usd != null ? (
            <>
              <Bound partial={total.partial.includes('cost_usd')}>{formatCost(total.cost_usd)}</Bound> with sub-agents
            </>
          ) : undefined
        }
        dashed
      />
    </div>
  )
}

/** The last line: what is left, how much work the conversation was, and
    the session id to copy. */
function Footer({
  snapshot,
  tree,
  sessionId,
}: {
  snapshot: ContextSnapshot
  tree: ConversationTree | null
  sessionId: string
}) {
  const self = tree?.self
  return (
    <div className="harness-ui-pop-footer">
      <span className="harness-ui-pop-footer-text">
        <span>
          <span className="harness-ui-strong">{formatTokens(Math.max(0, snapshot.free))}</span> tokens left
        </span>
        {self ? (
          <span>
            · {plural(self.turns, 'step')} · {plural(self.function_calls, 'call')}
            {self.function_call_errors ? ` · ${num(self.function_call_errors)} failed` : ''}
          </span>
        ) : null}
      </span>
      <CopySessionId sessionId={sessionId} />
    </div>
  )
}

/**
 * The window card: headline, bar, legend, the recent steps, three stats and
 * the footer line — the whole picture in one glance, in the order you ask
 * the questions: how full, of what, where it is going, what it cost.
 */
function WindowCard({
  snapshot,
  history,
  state,
  tree,
  liveCost,
  onRetry,
  sessionId,
}: {
  snapshot: ContextSnapshot
  history: HistoryState
  state: MetricsState
  tree: ConversationTree | null
  liveCost: number | null | undefined
  onRetry: () => void
  sessionId: string
}) {
  const usable = snapshot.usable
  const cats = categories(snapshot)
  const estimated = !snapshot.estimator || snapshot.estimator === 'heuristic'
  const pct = Math.round(Math.min(1, snapshot.total / usable) * 100)
  // One row per category: the conversation as a whole, not its parts.
  const rows = legendRows(snapshot, cats).filter((row) => !row.sub)
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
  const partial = tree ? treeTotals(tree, liveCost) : null
  const unreported = partial?.total?.unreported.length ? partial.total : partial?.self.partial.length ? partial.self : null
  return (
    <>
      <Hero total={snapshot.total} usable={usable} estimated={estimated} />
      <StackBar pct={pct}>
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
      </StackBar>
      <div className="harness-ui-legend">
        {rows.map((row) => (
          <LegendRow key={row.key} row={row} usable={usable} />
        ))}
      </div>
      <StepBars history={history} usable={usable} current={snapshot.total} />
      <StatTiles snapshot={snapshot} state={state} tree={tree} liveCost={liveCost} />
      {unreported && tree ? (
        <p className="harness-ui-note harness-ui-footnote">
          <Info size={16} aria-hidden />
          <span>≥ leaves out what {idList(unreported.unreported, tree.self.session_id)} did not report.</span>
        </p>
      ) : null}
      {failure}
      <Footer snapshot={snapshot} tree={tree} sessionId={sessionId} />
    </>
  )
}
/** The models a session or a group ran on, most steps first. */
function modelsLine(rows: SessionUsage[]) {
  const names = modelTotals(rows).map(modelLabel)
  return names.length > 2 ? `${names.slice(0, 2).join(', ')} +${names.length - 2} more` : names.join(', ')
}

/** The steps / calls / failures / context / age line of one session, led by
    its models when the tree mixes them. */
function sessionMeta(usage: SessionUsage, lead?: string, models = false) {
  const parts = lead ? [lead] : []
  if (models) parts.push(modelsLine([usage]))
  parts.push(plural(usage.turns, 'step'))
  if (usage.function_call_errors > 0) parts.push(`${num(usage.function_call_errors)} failed`)
  const context = usage.context
  if (context && context.usable > 0) parts.push(`context ${percentOf(context.total, context.usable)}`)
  if (context?.timestamp) parts.push(ago(context.timestamp))
  return parts.join(' · ')
}

/** The same line for a group: its size (and how many a filter left), steps,
    failures, unpriced rows, age. Steps and failures are the whole group's. */
function groupMeta(group: SubAgentGroup, models = false) {
  const shown = group.rows.length - 1
  const t = group.totals
  const parts = models ? [modelsLine(group.rows)] : []
  parts.push(
    shown < group.size ? `${num(shown)} of ${plural(group.size, 'sub-agent')}` : plural(group.size, 'sub-agent'),
    plural(t.turns, 'step'),
  )
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

/** More than one model anywhere in the conversation's subtree: only then
    do the sub-agent rows say which model each ran on. */
function mixesModels(tree: ConversationTree) {
  return modelTotals([tree.self, ...tree.subAgents.map((row) => row.usage)]).length > 1
}

/** One model in one column: its cost, then its steps (and, over the
    subtree, the sessions that ran it). Tokens without a cost read "no
    cost"; nothing reported reads "—". */
function ModelCell({ entry, sessions = false }: { entry?: ModelTotals; sessions?: boolean }) {
  if (!entry) return <>—</>
  const t = entry.totals
  const tokens = t.input_tokens != null || t.output_tokens != null || t.cache_read_tokens != null
  return (
    <span className="harness-ui-model-cell">
      <span data-unpriced={t.cost_usd == null ? '' : undefined}>
        {t.cost_usd != null ? (
          <Bound partial={t.partial.includes('cost_usd')}>{formatCost(t.cost_usd)}</Bound>
        ) : tokens ? (
          'no cost'
        ) : (
          '—'
        )}
      </span>
      <span className="harness-ui-model-steps">
        <span>{plural(t.turns, 'step')}</span>
        {sessions ? (
          <>
            {' · '}
            <span>{plural(entry.sessions, 'session')}</span>
          </>
        ) : null}
      </span>
    </span>
  )
}

/**
 * Which models did the work, in Usage's columns: this conversation's own
 * steps and, when it spawned sub-agents, its whole subtree's. `in use` marks
 * the live snapshot's model once there is more than one to tell apart.
 */
function ModelsSection({
  tree,
  current,
}: {
  tree: ConversationTree
  current?: { model: string; provider?: string }
}) {
  const headingId = useId()
  const own = modelTotals([tree.self])
  const subtree = tree.subAgents.length
    ? modelTotals([tree.self, ...tree.subAgents.map((row) => row.usage)])
    : null
  const rows = subtree ?? own
  // One model is already named in the head; the table earns its place
  // once there is a mix to tell apart.
  if (rows.length < 2) return null
  const ownByKey = new Map(own.map((entry) => [entry.key, entry]))
  const subCount = tree.subAgents.length
  const inUse = (entry: ModelTotals) =>
    rows.length > 1 &&
    current != null &&
    entry.model === current.model &&
    (!current.provider || entry.provider === current.provider)
  return (
    <section className="harness-ui-section" aria-labelledby={headingId}>
      <div className="harness-ui-section-head">
        <h3 id={headingId} className={uiClasses.eyebrow}>
          Models
        </h3>
        <span className="harness-ui-section-meta">{plural(rows.length, 'model')}</span>
      </div>
      <TableViewport>
        <Table className="harness-ui-table harness-ui-models" density="compact">
          {subtree ? (
            <TableHeader>
              <TableRow>
                <TableHead>Model</TableHead>
                <TableHead className="harness-ui-num">This conversation</TableHead>
                <TableHead className="harness-ui-num">With {plural(subCount, 'sub-agent')}</TableHead>
              </TableRow>
            </TableHeader>
          ) : null}
          <TableBody>
            {rows.map((entry) => (
              <TableRow key={entry.key}>
                <TableHead scope="row">
                  <span className="harness-ui-model">
                    <span className="harness-ui-model-name">
                      <span className="harness-ui-model-id">{modelLabel(entry)}</span>
                      {inUse(entry) ? <Chip>in use</Chip> : null}
                    </span>
                    <span className="harness-ui-model-provider">{entry.provider}</span>
                  </span>
                </TableHead>
                <TableCell className="harness-ui-num">
                  <ModelCell entry={ownByKey.get(entry.key)} />
                </TableCell>
                {subtree ? (
                  <TableCell className="harness-ui-num">
                    <ModelCell entry={entry} sessions />
                  </TableCell>
                ) : null}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableViewport>
    </section>
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
  const mixed = mixesModels(tree)
  return (
    <section className="harness-ui-section" aria-labelledby={headingId}>
      <div className="harness-ui-section-head">
        <h3 id={headingId} className={uiClasses.eyebrow}>
          Where the cost went
        </h3>
        <span className="harness-ui-section-meta">{nested ? `${plural(count, 'sub-agent')} · ${num(groups.length)} direct` : plural(count, 'sub-agent')}</span>
      </div>
      <div className="harness-ui-cost-list">
        <CostRow
          label="This conversation"
          mono={false}
          meta={[
            mixed ? modelsLine([tree.self]) : null,
            plural(self.turns, 'step'),
            plural(self.function_calls, 'call'),
            self.function_call_errors ? `${num(self.function_call_errors)} failed` : null,
          ]
            .filter(Boolean)
            .join(' · ')}
          cost={self.cost_usd}
          partial={self.partial.includes('cost_usd')}
          total={totalCost}
        />
        {ranked.map((group) => (
          <CostRow
            key={group.head.session_id}
            label={group.head.session_id}
            meta={group.rows.length > 1 ? groupMeta(group, mixed) : sessionMeta(group.head, undefined, mixed)}
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
  const mixed = mixesModels(tree)
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
                meta={nested ? groupMeta(group, mixed) : sessionMeta(group.head, undefined, mixed)}
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
                      description={sessionMeta(row, index === 0 ? 'spawned the rest' : undefined, mixed)}
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

/** The session id's copy button, for the card's footer: the label names
    the id, the flash says it copied, the live region says so aloud. */
function CopySessionId({ sessionId }: { sessionId: string }) {
  const { state, copy } = useCopyFlash(sessionId, 1400)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  useEffect(() => {
    if (state === 'idle') return
    setAnnouncement((prev) => ({
      seq: (prev?.seq ?? 0) + 1,
      text: state === 'copied' ? 'Session id copied' : "Couldn't copy the session id",
      urgency: state === 'copied' ? 'polite' : 'assertive',
    }))
  }, [state])
  const copied = state === 'copied'
  return (
    <>
      <IconButton
        label={copied ? 'Copied' : `Copy session id ${sessionId}`}
        tooltip={false}
        onClick={copy}
        data-copy={state}
        className="harness-ui-copy"
      >
        {copied ? <Check size={16} aria-hidden /> : <Copy size={16} aria-hidden />}
      </IconButton>
      <LiveRegion announcement={announcement} />
    </>
  )
}

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

/** The dialog's title and one-line descriptor: the model (and provider)
    this conversation is running on. */
function PopoverHead({
  model,
  provider,
  estimated,
}: {
  model: string
  provider?: string
  estimated?: boolean
}) {
  return (
    <div className="harness-ui-pop-head">
      <DialogTitle>Context usage</DialogTitle>
      <DialogDescription className="harness-ui-pop-model">
        <span className="harness-ui-pop-model-id">{model}</span>
        {provider ? <span className="harness-ui-pop-provider">· {provider}</span> : null}
        {estimated != null ? (
          <span className="harness-ui-pop-provider">· {estimated ? 'estimate' : 'exact'}</span>
        ) : null}
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
  history: HistoryState
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
  history,
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
  // A refresh that no longer carries this session (deleted mid-flight)
  // takes the sub-agents page away: fall back to the overview for good
  // rather than jump back to the page if a later answer carries it again.
  // Focus is the dialog's: Radix's focus scope refocuses its container when
  // the focused Back button unmounts.
  const hasTree = tree !== null
  useEffect(() => {
    if (view.page === 'sub-agents' && !hasTree) setView({ page: 'overview' })
  }, [view.page, hasTree])
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
  return (
    <div ref={root} className="harness-ui-pop">
      <PopoverHead
        model={live?.model || modelId || 'model'}
        provider={live?.provider}
        estimated={live ? !live.estimator || live.estimator === 'heuristic' : undefined}
      />
      {live ? (
        <WindowCard
          snapshot={live}
          history={history}
          state={metrics}
          tree={tree}
          liveCost={liveCost}
          onRetry={onRetry}
          sessionId={sessionId}
        />
      ) : (
        <EmptyWindow contextWindow={contextWindow} />
      )}
      {tree ? <ModelsSection tree={tree} current={live ?? undefined} /> : null}
      {tree ? (
        <CostSection
          tree={tree}
          liveCost={liveCost}
          onOpenSession={onOpenSession}
          onShowAll={(expand) => setView({ page: 'sub-agents', expand })}
          restoreFocus={view.page === 'overview' && view.returned === true}
        />
      ) : null}
      {!live || parentId ? (
        <div className="harness-ui-pop-foot">
          {!live ? <SessionIdRow sessionId={sessionId} /> : null}
          {parentId ? <ParentRow parentId={parentId} onOpenSession={onOpenSession} /> : null}
        </div>
      ) : null}
    </div>
  )
}

interface ContextChipSurfaceProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  triggerLabel: string
  trigger: ReactNode
  popover: ReactNode
  /** The usage tone, carried on the chip root so the stylesheet can colour
      the counts and the hover without a second colour map. */
  tone?: Tone
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
  tone,
}: ContextChipSurfaceProps) {
  const phone = usePhone()
  const Root = phone ? BottomSheet : Dialog
  const Trigger = phone ? BottomSheetTrigger : DialogTrigger
  return (
    <div className="harness-ui-chip" data-tone={tone}>
      <Root open={open} onOpenChange={onOpenChange}>
        <Trigger asChild>
          <button
            type="button"
            className="harness-ui-chip-btn"
            aria-label={triggerLabel}
            data-open={open ? '' : undefined}
          >
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
    const history = useContextHistory(host, sessionId, open, snapshot?.turn_id)
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
    const popover = (
      <ContextPopover
        snapshot={snapshot}
        modelId={modelId}
        contextWindow={contextWindow}
        sessionId={sessionId}
        metrics={metrics.state}
        history={history}
        onRetry={metrics.retry}
        onOpenSession={onOpenSession}
      />
    )

    /* The ring carries proportion; the counts carry the number you act on;
       the percentage keeps its job in the label, the tooltip and the
       breakdown's hero. */
    const ring = (ratio: number, pct: number, tone: Tone) => (
      <span
        className="harness-ui-chip-ring"
        role="progressbar"
        aria-label="context window usage"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <RingGauge ratio={ratio} size={16} stroke={2.5} color={TONE_COLOR[tone]} />
      </span>
    )
    const counts = (used: string, window: string) => (
      <span className="harness-ui-chip-counts">
        {used}
        <span className="harness-ui-chip-sep">/</span>
        {window}
      </span>
    )

    if (!snapshot || snapshot.usable <= 0) {
      if (contextWindow && contextWindow > 0) {
        return (
          <ContextChipSurface
            open={open}
            onOpenChange={setOpen}
            triggerLabel={`context: waiting for usage, ${contextWindow.toLocaleString()} token window — click for the breakdown`}
            trigger={
              <>
                {ring(0, 0, 'ok')}
                {counts('0', formatTokens(contextWindow))}
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
              {ring(0, 0, 'ok')}
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
        tone={tone}
        triggerLabel={`context: ${snapshot.total.toLocaleString()} of ${snapshot.usable.toLocaleString()} tokens (${pct}%) — click for the breakdown`}
        trigger={
          <>
            {ring(ratio, pct, tone)}
            {counts(formatTokens(snapshot.total), formatTokens(snapshot.usable))}
            {caret}
          </>
        }
        popover={popover}
      />
    )
  }
}
