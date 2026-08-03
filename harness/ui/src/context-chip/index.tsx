/**
 * The `context` session chip — a live view of the session's context window
 * matching the console's ContextUsage aesthetic (`ctx` label, bordered bar,
 * percent, `12.3k/200k` counts). Hydrates from `harness::metrics` on mount
 * and per session change; stays live over the worker's own
 * `harness::turn-completed` trigger (Message-path binding, GC'd with the
 * tab). Click toggles an anchored popover breaking the window down by
 * category with a stacked segment bar, legend, and last-turn actuals.
 */

import { useEffect, useRef, useState } from 'react'
import type { Host } from '@iii-dev/console-ui'
import { formatCost, formatTokens } from '../lib/format'
import {
  type ContextSnapshot,
  type SnapshotMessages,
  isSnapshot,
} from '../lib/metrics'

/** Per-tab handler ids (host.iii.on namespaces them `::<browserId>`). */
const EVENTS_FN = 'iii::harness-ui::events'
const STATE_FN = 'iii::harness-ui::ctx-state'

const WARN_THRESHOLD = 0.75
const DANGER_THRESHOLD = 0.9

export interface SessionChipProps {
  sessionId: string
  modelId?: string
  contextWindow?: number
}

interface TurnCompletedEvent {
  session_id?: string
  turn_id?: string
  terminal?: boolean
  context?: unknown
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

type Tone = 'ok' | 'warn' | 'alert'

const TONE_COLOR: Record<Tone, string> = {
  ok: 'var(--color-accent)',
  warn: 'var(--color-warn)',
  alert: 'var(--color-alert)',
}

function toneFor(ratio: number): Tone {
  if (ratio >= DANGER_THRESHOLD) return 'alert'
  if (ratio >= WARN_THRESHOLD) return 'warn'
  return 'ok'
}

const ink = (opacity: number) =>
  `color-mix(in srgb, var(--color-ink) ${opacity}%, transparent)`
const accent = (opacity: number) =>
  `color-mix(in srgb, var(--color-accent) ${opacity}%, transparent)`

const COLOR_SYSTEM = ink(80)
const COLOR_TOOLS = ink(55)
const COLOR_USER = accent(95)
const COLOR_ASSISTANT = accent(70)
const COLOR_RESULTS = accent(45)
const COLOR_HOOKS = ink(35)
const COLOR_OVERHEAD = ink(20)
const COLOR_FREE = 'var(--color-rule-2)'

const EMPTY_MESSAGES: SnapshotMessages = {
  user: 0,
  assistant: 0,
  function_result: 0,
  custom: 0,
}

function segments(snapshot: ContextSnapshot) {
  const cats = snapshot.categories
  const messages = cats.messages ?? EMPTY_MESSAGES
  return [
    { key: 'system', tokens: cats.system_prompt ?? 0, color: COLOR_SYSTEM },
    { key: 'tools', tokens: cats.tools ?? 0, color: COLOR_TOOLS },
    { key: 'user', tokens: messages.user ?? 0, color: COLOR_USER },
    { key: 'assistant', tokens: messages.assistant ?? 0, color: COLOR_ASSISTANT },
    {
      key: 'results',
      tokens: (messages.function_result ?? 0) + (messages.custom ?? 0),
      color: COLOR_RESULTS,
    },
    { key: 'hooks', tokens: cats.hook_guidance ?? 0, color: COLOR_HOOKS },
    { key: 'overhead', tokens: cats.overhead ?? 0, color: COLOR_OVERHEAD },
  ]
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
  const messages = snapshot.categories.messages ?? EMPTY_MESSAGES
  const conversation =
    (messages.user ?? 0) +
    (messages.assistant ?? 0) +
    (messages.function_result ?? 0) +
    (messages.custom ?? 0)
  const hookGuidance = snapshot.categories.hook_guidance ?? 0
  const free = snapshot.free ?? Math.max(0, usable - snapshot.total)
  const usage = snapshot.usage
  const hasActuals =
    usage != null && (usage.input != null || usage.cache_read != null)
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
        {segments(snapshot)
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
        <LegendRow
          color={COLOR_SYSTEM}
          label="System prompt"
          tokens={snapshot.categories.system_prompt ?? 0}
          usable={usable}
        />
        <LegendRow
          color={COLOR_TOOLS}
          label="Function schemas"
          tokens={snapshot.categories.tools ?? 0}
          usable={usable}
        />
        <LegendRow
          color={COLOR_ASSISTANT}
          label="Conversation"
          tokens={conversation}
          usable={usable}
        />
        {hookGuidance > 0 ? (
          <LegendRow
            color={COLOR_HOOKS}
            label="Hook guidance"
            tokens={hookGuidance}
            usable={usable}
          />
        ) : null}
        {snapshot.compacted ? (
          <LegendRow
            color={null}
            label="Summary"
            tokens={snapshot.summarized_head_tokens ?? 0}
            usable={usable}
            badge="compacted"
          />
        ) : null}
        <LegendRow
          color={COLOR_OVERHEAD}
          label="Overhead"
          tokens={snapshot.categories.overhead ?? 0}
          usable={usable}
        />
        <LegendRow color={COLOR_FREE} label="Free" tokens={free} usable={usable} />
      </div>
      <div className="harness-ui-pop-foot">
        <span>est. {snapshot.estimator ?? 'unknown'}</span>
        {hasActuals ? (
          <span>
            last turn actual{' '}
            {formatTokens((usage?.input ?? 0) + (usage?.cache_read ?? 0))} ·
            output {formatTokens(usage?.output ?? 0)}
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

    useEffect(() => {
      let cancelled = false
      setSnapshot(null)
      setOpen(false)
      host.iii
        .trigger('state::get', { scope: 'harness_context', key: sessionId })
        .then((value) => {
          if (cancelled) return
          if (isSnapshot(value) && value.session_id === sessionId)
            setSnapshot(value)
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
      const offHandler = host.iii.on<StateEvent>(STATE_FN, (event) => {
        if (!event || event.key !== sessionId) return
        if (event.event_type === 'state:deleted') return
        if (isSnapshot(event.new_value) && event.new_value.session_id === sessionId)
          setSnapshot(event.new_value)
      })
      const offTrigger = host.iii.registerTrigger({
        type: 'state',
        function_id: `${STATE_FN}::${host.iii.browserId}`,
        config: { scope: 'harness_context', key: sessionId },
      })
      return () => {
        offTrigger()
        offHandler()
      }
    }, [host, sessionId])

    useEffect(() => {
      const offHandler = host.iii.on<TurnCompletedEvent>(EVENTS_FN, (event) => {
        if (!event || event.session_id !== sessionId) return
        if (isSnapshot(event.context)) setSnapshot(event.context)
      })
      const offTrigger = host.iii.registerTrigger({
        type: 'harness::turn-completed',
        function_id: `${EVENTS_FN}::${host.iii.browserId}`,
        config: { session_id: sessionId },
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
