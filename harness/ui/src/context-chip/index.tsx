/**
 * The `context` session chip — a live view of the session's context window
 * matching the console's ContextUsage aesthetic (`ctx` label, bordered bar,
 * percent, `12.3k/200k` counts). Hydrates from the stored snapshot
 * (`state::get` on `harness_context/<session id>`) on mount and per session
 * change; stays live over the state worker's own `state` trigger for that
 * key (Message-path binding, GC'd with the tab), which fires on every
 * generate step. Click toggles an anchored popover breaking the window down
 * by category with a stacked segment bar, legend, and last-turn actuals.
 */

import { useEffect, useRef, useState } from 'react'
import type { Host } from '@iii-dev/console-ui'
import { formatCost, formatTokens } from '../lib/format'
import {
  type ContextSnapshot,
  type SnapshotUsage,
  isSnapshot,
} from '../lib/metrics'
import { TONE_COLOR, toneFor } from '../lib/tone'

/** Per-tab handler id (host.iii.on namespaces it `::<browserId>`). */
const STATE_FN = 'iii::harness-ui::ctx-state'

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

const ink = (opacity: number) =>
  `color-mix(in srgb, var(--color-ink) ${opacity}%, transparent)`
const accent = (opacity: number) =>
  `color-mix(in srgb, var(--color-accent) ${opacity}%, transparent)`

const COLOR_FREE = 'var(--color-rule-2)'

type CategoryKey =
  | 'system'
  | 'tools'
  | 'user'
  | 'assistant'
  | 'results'
  | 'hooks'
  | 'overhead'

interface Category {
  key: CategoryKey
  label: string
  color: string
  tokens: number
}

/** The three categories the legend shows as one "Conversation" row. */
const CONVERSATION_KEYS: CategoryKey[] = ['user', 'assistant', 'results']

/**
 * Every category of the assembled window, in bar order. The single source
 * for both the stacked bar (which draws each entry) and the legend (which
 * merges the conversation entries into one row).
 */
function categories(snapshot: ContextSnapshot): Category[] {
  const cats = snapshot.categories
  const messages = cats.messages
  return [
    {
      key: 'system',
      label: 'System prompt',
      color: ink(80),
      tokens: cats.system_prompt,
    },
    {
      key: 'tools',
      label: 'Function schemas',
      color: ink(55),
      tokens: cats.tools,
    },
    { key: 'user', label: 'User', color: accent(95), tokens: messages.user },
    {
      key: 'assistant',
      label: 'Assistant',
      color: accent(70),
      tokens: messages.assistant,
    },
    {
      key: 'results',
      label: 'Function results',
      color: accent(45),
      tokens: messages.function_result + messages.custom,
    },
    {
      key: 'hooks',
      label: 'Hook guidance',
      color: ink(35),
      // Optional on the wire (serde default): absent in snapshots written
      // before the category existed.
      tokens: cats.hook_guidance ?? 0,
    },
    {
      key: 'overhead',
      label: 'Overhead',
      color: ink(20),
      tokens: cats.overhead,
    },
  ]
}

/**
 * The prompt cache view of the last generation. Providers bill a cache read
 * at a fraction of fresh input and a cache write at a premium, so on a long
 * session the hit rate drives cost more than the window size does. `null`
 * when the provider reported no cache activity at all.
 */
function cacheSummary(usage: SnapshotUsage | undefined) {
  const read = usage?.cache_read ?? 0
  const write = usage?.cache_write ?? 0
  if (read === 0 && write === 0) return null
  const prompt = (usage?.input ?? 0) + read + write
  const hitPct = prompt > 0 ? Math.round((read / prompt) * 100) : 0
  return {
    read,
    write,
    hitPct,
    // A cold or broken prefix means the whole prompt is re-billed at the
    // write premium every turn, which is worth flagging rather than dimming.
    tone: hitPct >= 70 ? 'ok' : hitPct < 30 ? 'warn' : 'plain',
  }
}

interface LegendEntry {
  key: string
  label: string
  /** `null` renders an invisible swatch — the row is not a bar segment. */
  color: string | null
  tokens: number
  badge?: string
}

/**
 * The legend, derived from the same category array the bar draws: the three
 * conversation entries collapse into one row, an empty hook row is dropped,
 * and the two rows that are not bar segments (the compaction summary, free
 * space) take their place around the overhead row.
 */
function legendRows(
  snapshot: ContextSnapshot,
  cats: Category[],
): LegendEntry[] {
  const conversation = cats.filter((c) => CONVERSATION_KEYS.includes(c.key))
  const rows: LegendEntry[] = []
  for (const category of cats) {
    if (category.key === 'assistant') {
      rows.push({
        ...category,
        label: 'Conversation',
        tokens: conversation.reduce((sum, entry) => sum + entry.tokens, 0),
      })
      continue
    }
    if (CONVERSATION_KEYS.includes(category.key)) continue
    if (category.key === 'hooks' && category.tokens <= 0) continue
    if (category.key === 'overhead' && snapshot.compacted) {
      rows.push({
        key: 'summary',
        label: 'Summary',
        color: null,
        tokens: snapshot.summarized_head_tokens ?? 0,
        badge: 'compacted',
      })
    }
    rows.push(category)
  }
  rows.push({
    key: 'free',
    label: 'Free',
    color: COLOR_FREE,
    tokens: snapshot.free,
  })
  return rows
}

function LegendRow({
  color,
  label,
  tokens,
  usable,
  badge,
}: {
  color: string | null
  label: string
  tokens: number
  usable: number
  badge?: string
}) {
  const pct = usable > 0 ? Math.round((tokens / usable) * 100) : 0
  return (
    <div className="harness-ui-legend-row">
      <span
        className="harness-ui-swatch"
        style={color ? { background: color } : { visibility: 'hidden' }}
      />
      <span className="harness-ui-legend-label">{label}</span>
      {badge ? <span className="harness-ui-badge">{badge}</span> : null}
      <span className="harness-ui-legend-val">{formatTokens(tokens)}</span>
      <span className="harness-ui-legend-pct">{pct}%</span>
    </div>
  )
}

function ContextPopover({
  snapshot,
  modelId,
}: {
  snapshot: ContextSnapshot
  modelId?: string
}) {
  const usable = snapshot.usable
  const pct =
    usable > 0 ? Math.round(Math.min(1, snapshot.total / usable) * 100) : 0
  const cats = categories(snapshot)
  const usage = snapshot.usage
  const hasActuals =
    usage != null && (usage.input != null || usage.cache_read != null)
  const cache = cacheSummary(usage)
  return (
    <div className="harness-ui-pop" role="dialog" aria-label="context breakdown">
      <div className="harness-ui-pop-head">
        <span className="harness-ui-pop-model">
          {snapshot.model || modelId || 'model'}
        </span>
        <span className="harness-ui-pop-usage">
          {pct}% of {formatTokens(usable)}
        </span>
      </div>
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
          <LegendRow
            key={row.key}
            color={row.color}
            label={row.label}
            tokens={row.tokens}
            usable={usable}
            badge={row.badge}
          />
        ))}
      </div>
      <div className="harness-ui-pop-foot">
        <span>
          {!snapshot.estimator || snapshot.estimator === 'heuristic'
            ? `est. ${snapshot.estimator ?? 'unknown'}`
            : `exact · ${
                snapshot.estimator === 'provider'
                  ? 'provider tokenizer'
                  : snapshot.estimator
              }`}
        </span>
        {hasActuals ? (
          <span>
            last step {formatTokens(usage?.input ?? 0)} in · output{' '}
            {formatTokens(usage?.output ?? 0)}
          </span>
        ) : null}
        {cache ? (
          <span
            title={
              'cache read is billed at a fraction of fresh input; cache write ' +
              'carries a premium. Hit rate is the cached share of the prompt.'
            }
          >
            cache {formatTokens(cache.read)} read ·{' '}
            {formatTokens(cache.write)} write ·{' '}
            <span className="harness-ui-cache-hit" data-tone={cache.tone}>
              {cache.hitPct}% hit
            </span>
          </span>
        ) : null}
        {usage?.cost_usd != null ? (
          <span>cost {formatCost(usage.cost_usd)}</span>
        ) : null}
      </div>
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
    const rootRef = useRef<HTMLDivElement | null>(null)

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
        if (isSnapshot(event.new_value) && event.new_value.session_id === sessionId)
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

    useEffect(() => {
      if (!open) return
      const onPointerDown = (event: MouseEvent) => {
        const root = rootRef.current
        if (root && !root.contains(event.target as Node)) setOpen(false)
      }
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key === 'Escape') setOpen(false)
      }
      document.addEventListener('mousedown', onPointerDown)
      document.addEventListener('keydown', onKeyDown)
      return () => {
        document.removeEventListener('mousedown', onPointerDown)
        document.removeEventListener('keydown', onKeyDown)
      }
    }, [open])

    if (!snapshot || snapshot.usable <= 0) {
      if (contextWindow && contextWindow > 0) {
        return (
          <div
            className="harness-ui-chip"
            title={`no turn yet — model context window ${contextWindow.toLocaleString()} tokens`}
          >
            <span>ctx</span>
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
            <span className="harness-ui-chip-pct">0%</span>
            <span className="harness-ui-chip-counts">
              0/{formatTokens(contextWindow)}
            </span>
          </div>
        )
      }
      return (
        <div className="harness-ui-chip" title="no context snapshot yet">
          <span>ctx</span>
          <span className="harness-ui-chip-empty">—</span>
        </div>
      )
    }

    const ratio = Math.min(1, snapshot.total / snapshot.usable)
    const pct = Math.round(ratio * 100)
    const tone = toneFor(ratio)

    return (
      <div className="harness-ui-chip" ref={rootRef}>
        <button
          type="button"
          className="harness-ui-chip-btn"
          onClick={() => setOpen((value) => !value)}
          aria-expanded={open}
          title={`${snapshot.total.toLocaleString()} / ${snapshot.usable.toLocaleString()} tokens (${pct}%)`}
        >
          <span>ctx</span>
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
          <span
            className="harness-ui-chip-pct"
            style={{
              color: tone === 'ok' ? 'var(--color-ink)' : TONE_COLOR[tone],
            }}
          >
            {pct}%
          </span>
          <span className="harness-ui-chip-counts">
            {formatTokens(snapshot.total)}/{formatTokens(snapshot.usable)}
          </span>
        </button>
        {open ? <ContextPopover snapshot={snapshot} modelId={modelId} /> : null}
      </div>
    )
  }
}
