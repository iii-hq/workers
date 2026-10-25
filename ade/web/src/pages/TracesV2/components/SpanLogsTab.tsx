import { Clock } from 'lucide-react'
import { EmptyState } from '@/components/ui/EmptyState'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { formatPossibleJson } from '../lib/formatPossibleJson'
import { redactAttributeEntries } from '../lib/redactAttributes'
import type { VisualizationSpan } from '../lib/traceTransform'
import { toMs } from '../lib/traceTransform'
import { formatRelative, formatTimestamp } from '../lib/traceUtils'

interface SpanLogsTabProps {
  span: VisualizationSpan
  /**
   * The span's function-trigger redactor (`spanRawRedactor` in
   * functionTriggerFromSpan.ts). Event attributes — `iii.payload.json`
   * chief among them — pretty-print into a `<pre>` below; that is the same
   * payload the info tab's card redacts, one tab-click away otherwise.
   */
  redact?: (value: unknown) => unknown
}

export function SpanLogsTab({ span, redact }: SpanLogsTabProps) {
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
    <div className="flex flex-col gap-2 p-4 font-inter">
      {sortedEvents.map((event, index) => {
        const eventMs = toMs(event.timestamp_unix_nano)
        const offsetMs = eventMs - firstEventMs
        const isException =
          event.name === 'exception' || event.name?.startsWith('exception')
        const attrEntries = redactAttributeEntries(event.attributes, redact)

        return (
          <div
            key={`${event.name}-${event.timestamp_unix_nano}`}
            data-span-event-name={event.name}
            className={cn(
              'overflow-hidden rounded-lg border',
              isException
                ? 'border-alert/30 bg-alert-muted'
                : 'border-border bg-card',
            )}
          >
            <div className="px-3 py-2">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div
                    className={cn(
                      'font-mono text-xs font-semibold',
                      isException ? 'text-alert-strong' : 'text-foreground',
                    )}
                  >
                    {event.name}
                  </div>
                  <div className="flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground tabular-nums">
                    <span>{formatTimestamp(eventMs)}</span>
                    {index > 0 && (
                      <>
                        <span aria-hidden>&middot;</span>
                        <span>{formatRelative(offsetMs)}</span>
                      </>
                    )}
                  </div>
                </div>
                <span className="shrink-0 rounded-md bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground tabular-nums">
                  #{index + 1}
                </span>
              </div>
            </div>

            {attrEntries.length > 0 && (
              <div className="border-t border-border px-3 py-2">
                <div className="flex flex-col gap-1.5">
                  {attrEntries.map(([key, value]) => (
                    <EventAttributeRow key={key} attrKey={key} value={value} />
                  ))}
                </div>
              </div>
            )}
          </div>
        )
      })}

      <div className="pt-1 text-center font-mono text-[11px] text-muted-foreground tabular-nums">
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
        <span className="font-mono text-muted-foreground">{attrKey}</span>
        <div className="code-theme-editor max-h-80 overflow-auto overscroll-contain rounded-md border border-border">
          <JsonHighlight code={formatted} className="px-3 py-2" />
        </div>
      </div>
    )
  }
  return (
    <div
      className="flex items-start gap-2 text-[11px]"
      data-span-event-attribute={attrKey}
    >
      <span className="shrink-0 font-mono text-muted-foreground">
        {attrKey}
      </span>
      <span className="font-mono break-all text-foreground tabular-nums">
        {typeof value === 'object' ? JSON.stringify(value) : String(value)}
      </span>
    </div>
  )
}
