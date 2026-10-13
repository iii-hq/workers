/**
 * Live calls of one function: who called it, how long it took, what went in
 * and what came back — updating as the engine records spans.
 *
 * The old console could not show this. It exists here because the console
 * worker already streams: the `trace` trigger is a coalesced "spans changed"
 * tick, so the feed re-reads `engine::traces::spans` filtered to this
 * function's span name on each beat instead of polling a timer.
 *
 * Every row is replayable — the recorded input becomes the invoke editor's
 * body, which turns "this call failed in production" into one click.
 */

import {
  Button,
  Eyebrow,
  type Host,
  JsonHighlight,
  SegmentedControl,
  Skeleton,
  StatusDot,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { ChevronRight, RotateCcw } from 'lucide-react'
import { Fragment, useCallback, useState } from 'react'
import {
  type CallRecord,
  listCalls,
  useLiveSignals,
  useResource,
} from './engine'
import { pretty } from './schema'
import { ErrorNote, Note } from './widgets'

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

/** "3s ago" for the live meta lines; the shared clock decides the unit. */
export function agoLabel(atMs: number, nowMs: number): string {
  const relative = formatRelative(atMs, nowMs)
  return relative === 'just now' ? relative : `${relative} ago`
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
  const [filter, setFilter] = useState<'all' | 'failed'>('all')

  // Trace ticks are frequent under load, so this debounces harder than the
  // catalogue subscriptions do.
  useLiveSignals(host, ['trace'], calls.reload, { debounceMs: 1200 })

  if (calls.error) {
    return (
      <ErrorNote
        title="Couldn't read recent calls"
        call="engine::traces::spans"
        message={calls.error}
        onRetry={calls.reload}
      />
    )
  }
  if (calls.data === null) {
    return (
      <div
        className="console-catalog-calls-loading"
        role="status"
        aria-label="Reading recent calls"
      >
        {[0, 1, 2, 3, 4].map((i) => (
          <Skeleton key={i} className="row" />
        ))}
      </div>
    )
  }
  if (calls.data.length === 0) {
    return (
      <Note>
        No recorded calls yet. This list follows the trace stream, so a call
        made from anywhere (the agent, another worker, the Run tab) shows up
        here as it happens.
      </Note>
    )
  }

  const now = Date.now()
  const failures = calls.data.filter((c) => !c.ok).length
  const slowest = calls.data.reduce((max, c) => Math.max(max, c.durationMs), 0)
  const median = medianDuration(calls.data)
  // spanId can be empty or duplicated on some backends, and the row id keys
  // AND drives open state. Ids come from the unfiltered list (so the filter
  // cannot shift them) and repeats get a suffix (so twins never open together).
  const seen = new Map<string, number>()
  const rows = calls.data.map((call) => {
    const base = call.spanId || `${call.traceId}:${call.startedAtMs}`
    const repeat = seen.get(base) ?? 0
    seen.set(base, repeat + 1)
    return { call, rowId: repeat ? `${base}#${repeat}` : base }
  })
  const shown = filter === 'failed' ? rows.filter((row) => !row.call.ok) : rows

  return (
    <div className="console-catalog-activity">
      <div className="console-catalog-activity-bar">
        <SegmentedControl
          variant="radio"
          aria-label="Filter calls"
          value={filter}
          onChange={setFilter}
          options={[
            { value: 'all', label: `All ${calls.data.length}`, icon: false },
            { value: 'failed', label: `Failed ${failures}`, icon: false },
          ]}
        />
        <span className="console-catalog-activity-summary">
          median {formatDuration(median)} · slowest {formatDuration(slowest)}
        </span>
      </div>
      {shown.length === 0 ? (
        <Note>None of the last {calls.data.length} calls failed.</Note>
      ) : (
        <TableViewport className="console-catalog-calls">
          <TableFrame>
            <Table density="compact" aria-label="Recent calls">
              <TableHeader>
                <TableRow>
                  <TableHead className="status-column">
                    <span className="console-catalog-sr">Status</span>
                  </TableHead>
                  <TableHead>Time</TableHead>
                  <TableHead className="num">Duration</TableHead>
                  <TableHead className="input-column">Input</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {shown.map(({ call, rowId }) => (
                  <CallRow
                    key={rowId}
                    call={call}
                    now={now}
                    open={open === rowId}
                    onToggle={() =>
                      setOpen((prev) => (prev === rowId ? null : rowId))
                    }
                    onReplay={onReplay}
                  />
                ))}
              </TableBody>
            </Table>
          </TableFrame>
        </TableViewport>
      )}
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
    <Fragment>
      <TableRow
        interactive
        selected={open}
        className="console-catalog-call-row"
        aria-expanded={open}
        tabIndex={0}
        onClick={onToggle}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault()
            onToggle()
          }
        }}
      >
        <TableCell className="status-column">
          <StatusDot tone={call.ok ? 'ok' : 'alert'} />
          <span className="console-catalog-sr">
            {call.ok ? 'succeeded' : 'failed'}
          </span>
        </TableCell>
        <TableCell className="mono" title={agoLabel(call.startedAtMs, now)}>
          {clockTime(call.startedAtMs)}
        </TableCell>
        <TableCell className="mono num">
          {formatDuration(call.durationMs)}
        </TableCell>
        <TableCell className="mono faint input-column">
          <span className="console-catalog-call-input">
            <span className="text">
              {call.input === undefined ? '—' : oneLine(call.input)}
            </span>
            <ChevronRight
              className="iii-ui-icon caret"
              data-open={open}
              aria-hidden
            />
          </span>
        </TableCell>
      </TableRow>
      {open ? (
        <TableRow className="console-catalog-call-detail" selected>
          <TableCell colSpan={4}>
            <div className="console-catalog-call-body">
              <div className="call-actions">
                {call.input !== undefined ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    type="button"
                    onClick={() => onReplay(withoutInjected(call.input))}
                  >
                    <RotateCcw aria-hidden />
                    Replay in Run
                  </Button>
                ) : null}
                <span className="console-catalog-hint">
                  trace {call.traceId}
                </span>
              </div>
              <div className="call-payloads">
                <div className="payload">
                  <Eyebrow as="div">Input</Eyebrow>
                  <JsonHighlight
                    code={
                      call.input === undefined
                        ? '(not recorded)'
                        : pretty(call.input)
                    }
                    className="console-catalog-json"
                    wrap
                  />
                </div>
                <div className="payload">
                  <Eyebrow as="div">{call.ok ? 'Output' : 'Error'}</Eyebrow>
                  <JsonHighlight
                    code={
                      call.output === undefined
                        ? '(not recorded)'
                        : pretty(call.output)
                    }
                    className="console-catalog-json"
                    wrap
                  />
                </div>
              </div>
            </div>
          </TableCell>
        </TableRow>
      ) : null}
    </Fragment>
  )
}

function oneLine(value: unknown): string {
  const flat = (pretty(withoutInjected(value)) || '').replace(/\s+/g, ' ')
  return flat.length > 120 ? `${flat.slice(0, 117)}…` : flat
}

function medianDuration(calls: CallRecord[]): number {
  const sorted = calls.map((c) => c.durationMs).sort((a, b) => a - b)
  const mid = Math.floor(sorted.length / 2)
  if (sorted.length === 0) return 0
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2
}
