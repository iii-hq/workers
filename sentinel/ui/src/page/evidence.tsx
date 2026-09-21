import {
  Chip,
  CodeHighlight,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  EmptyState,
  Eyebrow,
  Skeleton,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import type { Host } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import type { Client, EvidenceBundle, EvidenceSpan, OccurrenceSummary } from '../api'

interface Props {
  api: Client
  host: Host
  occurrence: OccurrenceSummary
  repositoryPath: string | null
}

/** How much of one attribute value is worth reading inline. The worker caps
    what it stores; this caps what the page puts on screen at once. */
const VALUE_PREVIEW = 600

/** An attribute longer than this, or a stack trace of any length, is filed
    under the disclosure rather than pushed between the reader and the tree. */
const INLINE_MAX = 400
const isBulky = ([key, value]: [string, string]) =>
  key.endsWith('stacktrace') || value.length > INLINE_MAX

/**
 * The frozen bundle. It is a snapshot by design — by the time somebody opens
 * a group the engine's ring has usually dropped the trace — so the view says
 * so rather than pretending to be live.
 *
 * What is shown first is the failure: the exception the span carried. The
 * SDK's own request and response events are the raw payloads around it, kept
 * one click away rather than dropped, because they are frequently where the
 * answer is and always where the noise is.
 */
export function EvidenceView({ api, occurrence, repositoryPath }: Props) {
  const [bundle, setBundle] = useState<EvidenceBundle | null>(null)
  const [pruned, setPruned] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    setBundle(null)
    setError(null)
    api
      .evidence(occurrence.id)
      .then((response) => {
        if (!live) return
        setBundle(response.evidence ?? null)
        setPruned(response.pruned)
      })
      .catch((cause) => live && setError(errorMessage(cause)))
    return () => {
      live = false
    }
  }, [api, occurrence.id])

  if (error) {
    return <EmptyState compact title="Could not read the evidence" description={error} />
  }
  if (pruned) {
    return (
      <EmptyState
        compact
        title="The evidence was pruned"
        description="Retention kept the occurrence and let its bundle go. The first occurrence, the most recent ones and one per worker version still have theirs."
      />
    )
  }
  if (!bundle) return <Skeleton />

  const origin = bundle.spans.find((span) => span.span_id === bundle.origin_span_id)
  const events = origin?.events ?? []
  const headline = events
    .filter((event) => event.name === 'exception')
    .flatMap((event) => Object.entries(event.attributes).filter((entry) => !isBulky(entry)))
  const bulky = events.flatMap((event) =>
    Object.entries(event.attributes)
      .filter((entry) => event.name !== 'exception' || isBulky(entry))
      .map(([key, value]) => [event.name === 'exception' ? key : `${event.name} · ${key}`, value] as const),
  )

  return (
    <div className="sentinel-ui-evidence">
      <div className="sentinel-ui-evidence-head">
        <Eyebrow>trace {bundle.trace_id.slice(0, 12)}</Eyebrow>
        {bundle.settled ? null : <Chip tone="warning">captured while still open</Chip>}
        {occurrence.worker_version ? <Chip tone="neutral">{occurrence.worker_version}</Chip> : null}
        {bundle.truncated.spans > 0 ? (
          <Chip tone="neutral">{bundle.truncated.spans} spans dropped for size</Chip>
        ) : null}
      </div>

      {origin?.status_description ? (
        <pre className="sentinel-ui-message">{origin.status_description}</pre>
      ) : null}

      {headline.length > 0 ? (
        <div className="sentinel-ui-event">
          {headline.map(([key, value]) => (
            <Attribute key={key} name={key} value={value} />
          ))}
        </div>
      ) : null}

      {bulky.length > 0 ? (
        <CollapsibleCard>
          <CollapsibleCardTrigger>
            <Eyebrow>stack trace and payloads</Eyebrow>
          </CollapsibleCardTrigger>
          <CollapsibleCardContent>
            <div className="sentinel-ui-event">
              {bulky.map(([key, value]) => (
                <Attribute key={key} name={key} value={value} />
              ))}
            </div>
          </CollapsibleCardContent>
        </CollapsibleCard>
      ) : null}

      <Eyebrow size="lg">span tree</Eyebrow>
      <ol className="sentinel-ui-spans">
        {bundle.spans.map((span) => (
          <SpanRow
            key={span.span_id}
            span={span}
            origin={span.span_id === bundle.origin_span_id}
            carried={bundle.propagated_through.includes(span.span_id)}
          />
        ))}
      </ol>

      {bundle.logs.length > 0 ? (
        <>
          <Eyebrow size="lg">logs of this trace</Eyebrow>
          <CodeHighlight
            code={bundle.logs.map((log) => `${log.severity_text.padEnd(5)} ${log.body}`).join('\n')}
            language="text"
            wrap
          />
        </>
      ) : null}

      {repositoryPath ? (
        <Eyebrow className="sentinel-ui-repo">code under {repositoryPath}</Eyebrow>
      ) : null}
    </div>
  )
}

function Attribute({ name, value }: { name: string; value: string }) {
  const long = value.length > VALUE_PREVIEW
  return (
    <div className="sentinel-ui-attribute">
      <span className="sentinel-ui-attribute-key">{name}</span>
      <CodeHighlight
        code={long ? `${value.slice(0, VALUE_PREVIEW)}\n…` : value}
        language="text"
        wrap
      />
      {long ? (
        <Eyebrow className="sentinel-ui-note">
          {value.length - VALUE_PREVIEW} more characters in sentinel::evidence::get
        </Eyebrow>
      ) : null}
    </div>
  )
}

function SpanRow({
  span,
  origin,
  carried,
}: {
  span: EvidenceSpan
  origin: boolean
  carried: boolean
}) {
  return (
    <li
      className="sentinel-ui-span"
      style={{ paddingLeft: `${Math.min(span.depth, 8) * 14}px` }}
      data-origin={origin ? 'true' : undefined}
    >
      <span className="sentinel-ui-span-name">{span.name}</span>
      <span className="sentinel-ui-span-service">{span.service_name}</span>
      {origin ? <Chip tone="danger">where it failed</Chip> : null}
      {carried ? <Chip tone="neutral">carried up</Chip> : null}
      {span.end_time_unix_nano === 0 ? <Chip tone="warning">still open</Chip> : null}
    </li>
  )
}
