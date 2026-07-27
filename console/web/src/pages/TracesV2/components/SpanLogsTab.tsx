import { Clock } from 'lucide-react'
import { EmptyState } from '@/components/ui/EmptyState'
import { formatPossibleJson } from '../lib/formatPossibleJson'
import type { VisualizationSpan } from '../lib/traceTransform'
import { toMs } from '../lib/traceTransform'
import { formatRelative, formatTimestamp } from '../lib/traceUtils'

interface SpanLogsTabProps {
  span: VisualizationSpan
}

export function SpanLogsTab({ span }: SpanLogsTabProps) {
  const sortedEvents = [...(span.events || [])].sort(
    (a, b) => a.timestamp_unix_nano - b.timestamp_unix_nano,
  )

  if (sortedEvents.length === 0) {
    return (
      <div className="p-4">
        <EmptyState
          icon={Clock}
          title="no events recorded"
          description="this span has no logged events or exceptions"
        />
      </div>
    )
  }

  const firstEventMs = toMs(sortedEvents[0].timestamp_unix_nano)

  return (
    <div className="p-5 space-y-2">
      {sortedEvents.map((event, index) => {
        const eventMs = toMs(event.timestamp_unix_nano)
        const offsetMs = eventMs - firstEventMs
        const isException =
          event.name === 'exception' || event.name?.startsWith('exception')
        const attrEntries = event.attributes
          ? Object.entries(event.attributes)
          : []

        return (
          <div
            key={`${event.name}-${event.timestamp_unix_nano}`}
            data-span-event-name={event.name}
            className={`border bg-bg ${
              isException
                ? 'border-l-2 border-l-alert border-y-rule border-r-rule bg-alert/5'
                : 'border-rule'
            }`}
          >
            <div className="px-4 py-3">
              <div className="flex items-start justify-between gap-3">
                <div className="flex-1 min-w-0">
                  <div
                    className={`font-mono text-[13px] font-semibold mb-0.5 lowercase ${
                      isException ? 'text-alert' : 'text-ink'
                    }`}
                  >
                    {event.name}
                  </div>
                  <div className="flex items-center gap-2 font-mono text-[10px] text-ink-faint tabular-nums">
                    <span>{formatTimestamp(eventMs)}</span>
                    {index > 0 && (
                      <>
                        <span className="text-ink-ghost">&middot;</span>
                        <span className="text-ink-faint">
                          {formatRelative(offsetMs)}
                        </span>
                      </>
                    )}
                  </div>
                </div>
                <span className="font-mono text-[10px] text-ink-faint flex-shrink-0 px-1.5 py-0.5 border border-rule bg-panel tabular-nums">
                  #{index + 1}
                </span>
              </div>
            </div>

            {attrEntries.length > 0 && (
              <div className="border-t border-rule px-4 py-2.5">
                <div className="space-y-1.5">
                  {attrEntries.map(([key, value]) => (
                    <EventAttributeRow key={key} attrKey={key} value={value} />
                  ))}
                </div>
              </div>
            )}
          </div>
        )
      })}

      <div className="font-mono text-[10px] text-ink-ghost text-center pt-2 lowercase tabular-nums">
        {sortedEvents.length} event{sortedEvents.length !== 1 ? 's' : ''}
      </div>
    </div>
  )
}

/**
 * Render one event attribute row. When the value is a JSON-encoded string
 * (typically `iii.payload.json` written by the iii-sdk auto-capture), parse
 * and pretty-print it inside a <pre> block. Everything else renders as a
 * single-line label/value pair.
 *
 * Detection lives in `lib/formatPossibleJson.ts` so the heuristic is testable
 * in isolation and reusable for any other JSON-in-attribute surface.
 */
function EventAttributeRow({
  attrKey,
  value,
}: {
  attrKey: string
  value: unknown
}) {
  const formatted = formatPossibleJson(value)
  if (formatted !== null) {
    return (
      <div
        className="flex flex-col gap-1 text-[11px]"
        data-span-event-attribute={attrKey}
      >
        <span className="font-mono text-ink-faint uppercase tracking-[0.06em] text-[10px]">
          {attrKey}
        </span>
        <pre className="border border-rule bg-bg px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink overflow-x-auto whitespace-pre">
          {formatted}
        </pre>
      </div>
    )
  }
  return (
    <div
      className="flex items-start gap-2 text-[11px]"
      data-span-event-attribute={attrKey}
    >
      <span className="font-mono text-ink-faint uppercase tracking-[0.06em] text-[10px] flex-shrink-0">
        {attrKey}
      </span>
      <span className="font-mono text-ink break-all tabular-nums">
        {typeof value === 'object' ? JSON.stringify(value) : String(value)}
      </span>
    </div>
  )
}
