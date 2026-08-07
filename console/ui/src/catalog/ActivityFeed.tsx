/**
 * Live calls of one function: who called it, how long it took, what went in
 * and what came back — updating as the engine records spans.
 *
 * The old console could not show this. It exists here because the console
 * worker already streams: the `trace` trigger is a coalesced "spans changed"
 * tick, so the feed re-reads `engine::traces::list` filtered to this
 * function's span name on each beat instead of polling a timer.
 *
 * Every row is replayable — the recorded input becomes the invoke editor's
 * body, which turns "this call failed in production" into one click.
 */

import { Button, type Host, JsonHighlight } from '@iii-dev/console-ui'
import { useCallback, useState } from 'react'
import {
  type CallRecord,
  listCalls,
  useLiveSignals,
  useResource,
} from './engine'
import { pretty } from './schema'
import { Note } from './widgets'

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour12: false })
}

/**
 * Bus calls are routinely tens of microseconds, so a fixed `ms` scale prints
 * a wall of `0.0ms` and hides the only number on the row that varies. Same
 * adaptive scale the traces page uses.
 */
export function formatDuration(ms: number): string {
  if (ms < 1) return `${Math.round(ms * 1000)}µs`
  if (ms < 1000) return `${ms.toFixed(1)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

/**
 * Fields the ENGINE adds to a payload on its way through the bus, not fields
 * the caller sent. Replaying them verbatim would put another worker's id on
 * the call, so they are dropped and the editor opens on what a caller would
 * actually type. The feed still displays the recorded input in full.
 */
function withoutInjected(input: unknown): unknown {
  if (typeof input !== 'object' || input === null || Array.isArray(input)) {
    return input
  }
  const copy: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(input)) {
    if (key === '_caller_worker_id') continue
    copy[key] = value
  }
  return copy
}

function ago(ms: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000))
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  return `${Math.round(minutes / 60)}h ago`
}

export function ActivityFeed({
  host,
  functionId,
  onReplay,
}: {
  host: Host
  functionId: string
  /** Push a recorded input back into the invoke editor. */
  onReplay: (input: unknown) => void
}) {
  const load = useCallback(
    () => listCalls(host, functionId),
    [host, functionId],
  )
  const calls = useResource(load)
  const [open, setOpen] = useState<string | null>(null)

  // Trace ticks are frequent under load, so this debounces harder than the
  // catalogue subscriptions do.
  useLiveSignals(host, ['trace'], calls.reload, { debounceMs: 1200 })

  if (calls.error) {
    return (
      <div className="console-catalog-error">
        engine::traces::list failed — {calls.error}
      </div>
    )
  }
  if (calls.data === null) return <Note>reading recent calls…</Note>
  if (calls.data.length === 0) {
    return (
      <Note>
        no recorded calls. This feed follows the trace stream, so a call made
        from anywhere — the agent, another worker, the invoke tab — appears here
        as it happens.
      </Note>
    )
  }

  const now = Date.now()
  const failures = calls.data.filter((c) => !c.ok).length
  const slowest = calls.data.reduce((max, c) => Math.max(max, c.durationMs), 0)
  const median = medianDuration(calls.data)

  return (
    <div className="console-catalog-activity">
      <div className="console-catalog-activity-summary">
        <span>{calls.data.length} recent calls</span>
        <span>median {formatDuration(median)}</span>
        <span>slowest {formatDuration(slowest)}</span>
        <span className={failures ? 'console-catalog-invalid' : undefined}>
          {failures} failed
        </span>
      </div>
      {calls.data.map((call, i) => {
        // spanId can be empty or duplicated on some backends — the row id
        // keys AND drives open state, so a collision would open every twin.
        const rowId = call.spanId || `${call.traceId}:${call.startedAtMs}:${i}`
        return (
          <CallRow
            key={rowId}
            call={call}
            now={now}
            open={open === rowId}
            onToggle={() => setOpen((prev) => (prev === rowId ? null : rowId))}
            onReplay={onReplay}
          />
        )
      })}
    </div>
  )
}

function CallRow({
  call,
  now,
  open,
  onToggle,
  onReplay,
}: {
  call: CallRecord
  now: number
  open: boolean
  onToggle: () => void
  onReplay: (input: unknown) => void
}) {
  return (
    <div className="console-catalog-call" data-open={open}>
      <button
        type="button"
        className="console-catalog-call-head"
        onClick={onToggle}
      >
        <span className="dot" data-ok={call.ok} />
        <span className="time">{clockTime(call.startedAtMs)}</span>
        <span className="ago">{ago(call.startedAtMs, now)}</span>
        <span className="worker">{call.worker}</span>
        <span className="duration">{formatDuration(call.durationMs)}</span>
      </button>
      {open ? (
        <div className="console-catalog-call-body">
          <div className="console-catalog-field-label">
            input
            {call.input !== undefined ? (
              <Button
                variant="pill"
                size="sm"
                onClick={() => onReplay(withoutInjected(call.input))}
              >
                replay
              </Button>
            ) : null}
          </div>
          <JsonHighlight
            code={
              call.input === undefined ? '(not recorded)' : pretty(call.input)
            }
            className="console-catalog-json"
            wrap
          />
          <div className="console-catalog-field-label">output</div>
          <JsonHighlight
            code={
              call.output === undefined ? '(not recorded)' : pretty(call.output)
            }
            className="console-catalog-json"
            wrap
          />
          <span className="console-catalog-hint">trace {call.traceId}</span>
        </div>
      ) : null}
    </div>
  )
}

function medianDuration(calls: CallRecord[]): number {
  const sorted = calls.map((c) => c.durationMs).sort((a, b) => a - b)
  const mid = Math.floor(sorted.length / 2)
  if (sorted.length === 0) return 0
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2
}
