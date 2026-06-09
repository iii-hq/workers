import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from 'react'
import { Button } from '@/components/ui/Button'
import type { StreamEvent } from '@/lib/backend'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'

interface TaggedEvent extends Record<string, unknown> {
  kind: StreamEvent['kind']
  /** monotonic counter so list keys stay unique even with duplicate timestamps */
  seq: number
  ts: number
}

export interface EventLogHandle {
  push: (event: StreamEvent) => void
  clear: () => void
}

interface EventLogProps {
  /** when null/undefined, the log is collapsed to a vertical strip */
  open: boolean
  onToggle: () => void
}

export const EventLog = forwardRef<EventLogHandle, EventLogProps>(
  ({ open, onToggle }, ref) => {
    const [events, setEvents] = useState<TaggedEvent[]>([])
    const seqRef = useRef(0)

    useImperativeHandle(
      ref,
      () => ({
        push: (event) => {
          seqRef.current += 1
          setEvents((prev) => [
            ...prev,
            { ...event, seq: seqRef.current, ts: Date.now() },
          ])
        },
        clear: () => setEvents([]),
      }),
      [],
    )

    const handleClear = useCallback(() => setEvents([]), [])

    if (!open) {
      return (
        <button
          type="button"
          onClick={onToggle}
          aria-label="open event log"
          className={cn(
            'w-7 shrink-0 border-l border-rule bg-bg flex items-start justify-center pt-3',
            'font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint hover:text-ink',
            'transition-colors',
          )}
        >
          <span className="[writing-mode:vertical-rl] rotate-180">
            event log
          </span>
        </button>
      )
    }

    return (
      <aside className="w-[260px] shrink-0 border-l border-rule bg-bg flex flex-col min-h-0">
        <header className="flex items-center justify-between px-3 py-2 border-b border-rule">
          <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
            event log
          </span>
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-[11px]"
              onClick={handleClear}
              aria-label="clear event log"
              disabled={events.length === 0}
            >
              clear
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-[11px]"
              onClick={onToggle}
              aria-label="hide event log"
            >
              hide
            </Button>
          </div>
        </header>
        <EventList events={events} />
      </aside>
    )
  },
)
EventLog.displayName = 'EventLog'

function EventList({ events }: { events: TaggedEvent[] }) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: events.length is the trigger.
  useEffect(() => {
    const c = containerRef.current
    if (!c) return
    const distanceFromBottom = c.scrollHeight - c.scrollTop - c.clientHeight
    if (distanceFromBottom < 80) {
      bottomRef.current?.scrollIntoView({ block: 'end' })
    }
  }, [events.length])

  if (events.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center px-4 py-6">
        <div className="font-mono text-[12px] text-ink-ghost lowercase text-center">
          no events yet. pick a scenario and send a message to populate.
        </div>
      </div>
    )
  }

  return (
    <div ref={containerRef} className="flex-1 overflow-y-auto">
      <ul className="flex flex-col">
        {events.map((e) => (
          <EventRow key={e.seq} event={e} />
        ))}
        <div ref={bottomRef} />
      </ul>
    </div>
  )
}

function EventRow({ event }: { event: TaggedEvent }) {
  const { kind, seq: _seq, ts, ...rest } = event
  const tone = toneFor(kind)
  const time = formatTime(ts)
  const hasPayload = Object.keys(rest).length > 0
  const summary = summarize(kind, rest)

  return (
    <li className="border-b border-rule-2 px-3 py-1.5 font-mono text-[11.5px] leading-[1.5]">
      <div className="flex items-baseline gap-2">
        <span className="text-ink-ghost tabular-nums shrink-0">{time}</span>
        <span className={cn('shrink-0', tone)}>{kind}</span>
        {summary ? (
          <span className="text-ink-faint truncate min-w-0">{summary}</span>
        ) : null}
      </div>
      {hasPayload && shouldExpandPayload(kind) ? (
        <div className="mt-1">
          <JsonHighlight code={JSON.stringify(rest, null, 2)} />
        </div>
      ) : null}
    </li>
  )
}

function toneFor(kind: StreamEvent['kind']): string {
  switch (kind) {
    case 'thought-start':
    case 'thought-end':
      return 'text-ink-faint'
    case 'thought-token':
      return 'text-ink-ghost'
    case 'fcall-start':
    case 'fcall-end':
    case 'fcall-approval-cleared':
      return 'text-accent'
    case 'assistant-token':
      return 'text-ink-ghost'
    case 'assistant-end':
      return 'text-ink'
    case 'compaction':
      return 'text-warn'
    case 'stop-reason':
      return 'text-warn'
  }
}

function shouldExpandPayload(kind: StreamEvent['kind']): boolean {
  return kind === 'fcall-start' || kind === 'fcall-end'
}

function summarize(
  kind: StreamEvent['kind'],
  rest: Record<string, unknown>,
): string | null {
  if (kind === 'thought-token' || kind === 'assistant-token') {
    const t = rest.token
    if (typeof t !== 'string') return null
    const visible = t.replace(/\s+/g, ' ').trim()
    return visible ? `“${truncate(visible, 32)}”` : '·whitespace·'
  }
  if (kind === 'thought-end' || kind === 'fcall-end') {
    const d = rest.durationMs
    if (typeof d === 'number') return `${d}ms`
  }
  if (kind === 'fcall-start') {
    const id = rest.functionId
    return typeof id === 'string' ? id : null
  }
  return null
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n)}…` : s
}

function formatTime(ts: number): string {
  const d = new Date(ts)
  const ms = String(d.getMilliseconds()).padStart(3, '0')
  return `${d.getMinutes().toString().padStart(2, '0')}:${d
    .getSeconds()
    .toString()
    .padStart(2, '0')}.${ms}`
}
