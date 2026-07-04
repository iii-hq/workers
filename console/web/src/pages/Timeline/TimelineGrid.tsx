import { cn } from '@/lib/utils'
import type { CommEvent } from '@/types/iii-agent-event'

interface TimelineGridProps {
  rootId: string
  lanes: string[]
  events: CommEvent[]
  /** Resolve a session id to a display title (falls back to the id). */
  laneTitle: (sessionId: string) => string
  onOpenSession: (sessionId: string) => void
}

const KIND_GLYPH: Record<CommEvent['kind'], string> = {
  spawn: '→',
  result: '⇠',
  notify: '~▸',
  trigger_fire: '⚡',
}

const KIND_LABEL: Record<CommEvent['kind'], string> = {
  spawn: 'spawn',
  result: 'result',
  notify: 'notify',
  trigger_fire: 'fire',
}

function timeOf(at: number): string {
  return new Date(at).toLocaleTimeString(undefined, { hour12: false })
}

/**
 * Sequence view: one column per session lane, one row per comm event, time
 * flowing down. The connector spans from-lane → to-lane; trigger fires with
 * no source render as a badge on the target lane.
 */
export function TimelineGrid({
  rootId,
  lanes,
  events,
  laneTitle,
  onOpenSession,
}: TimelineGridProps) {
  const template = `90px repeat(${lanes.length}, minmax(120px, 1fr))`
  return (
    <div className="overflow-x-auto">
      <div className="grid gap-y-px min-w-fit" style={{ gridTemplateColumns: template }}>
        {/* header */}
        <div className="px-2 py-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost">
          time
        </div>
        {lanes.map((id) => (
          <button
            key={id}
            type="button"
            onClick={() => onOpenSession(id)}
            title={id}
            className={cn(
              'px-2 py-2 font-mono text-[11px] lowercase truncate text-left border-b border-rule',
              id === rootId ? 'text-ink' : 'text-ink-faint hover:text-ink',
            )}
          >
            {laneTitle(id)}
          </button>
        ))}
        {/* rows */}
        {events.map((e, idx) => {
          const fromIdx = e.from ? lanes.indexOf(e.from.session_id) : -1
          const toId = e.to?.session_id ?? e.trigger?.child_session_id
          const toIdx = toId ? lanes.indexOf(toId) : -1
          const anchor = toIdx >= 0 ? toIdx : fromIdx >= 0 ? fromIdx : 0
          const lo = fromIdx >= 0 && toIdx >= 0 ? Math.min(fromIdx, toIdx) : anchor
          const hi = fromIdx >= 0 && toIdx >= 0 ? Math.max(fromIdx, toIdx) : anchor
          const leftward = fromIdx >= 0 && toIdx >= 0 && toIdx < fromIdx
          return (
            <div
              // seq is unique per family log; seq-0 (failed-append live
              // events) fall back to the index to avoid key collisions.
              key={e.seq !== 0 ? `s${e.seq}` : `u${idx}-${e.at}`}
              className="contents"
            >
              <div className="px-2 py-1.5 font-mono text-[10px] text-ink-ghost tabular-nums">
                {timeOf(e.at)}
              </div>
              {lanes.map((laneId, i) => {
                const inSpan = i >= lo && i <= hi
                const isAnchor = i === anchor
                // Spec click-through: spawn/notify/result rows open the
                // session the event points at; trigger fires jump to the
                // Triggers view (no per-trigger detail view exists yet).
                const targetSession = toId ?? e.from?.session_id
                const onRowClick =
                  e.kind === 'trigger_fire'
                    ? () => {
                        window.location.hash = '#/triggers'
                      }
                    : targetSession
                      ? () => onOpenSession(targetSession)
                      : undefined
                return (
                  <div
                    key={laneId}
                    className={cn(
                      'px-2 py-1.5 font-mono text-[11px] min-w-0',
                      inSpan && !isAnchor && 'border-b border-dashed border-rule-2',
                    )}
                  >
                    {isAnchor ? (
                      <button
                        type="button"
                        onClick={onRowClick}
                        disabled={!onRowClick}
                        title={
                          e.kind === 'trigger_fire'
                            ? 'open triggers view'
                            : targetSession
                              ? `open session ${targetSession}`
                              : undefined
                        }
                        className={cn(
                          'flex items-baseline gap-1.5 min-w-0 w-full text-left',
                          onRowClick
                            ? 'hover:bg-paper-2 focus-visible:bg-paper-2 focus-visible:outline-none transition-colors'
                            : 'cursor-default',
                        )}
                      >
                        <span
                          className={cn(
                            'shrink-0',
                            e.kind === 'trigger_fire' ? 'text-warn' : 'text-accent',
                          )}
                          aria-hidden
                        >
                          {leftward ? '←' : KIND_GLYPH[e.kind]}
                        </span>
                        <span className="uppercase tracking-[0.06em] text-[10px] text-ink-faint shrink-0">
                          {KIND_LABEL[e.kind]}
                          {e.trigger ? `·${e.trigger.action}` : ''}
                        </span>
                        {e.summary ? (
                          <span className="text-ink-faint truncate" title={e.summary}>
                            {e.summary}
                          </span>
                        ) : null}
                      </button>
                    ) : null}
                  </div>
                )
              })}
            </div>
          )
        })}
      </div>
      {events.length === 0 ? (
        <div className="px-3 py-6 font-mono text-[12.5px] text-ink-ghost">
          · no communication recorded for this family yet
        </div>
      ) : null}
    </div>
  )
}
