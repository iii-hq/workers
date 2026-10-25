import { ExternalLink, Link2 } from 'lucide-react'
import { useMemo } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
import type { VisualizationSpan } from '../lib/traceTransform'

/**
 * No `redact` prop, deliberately: this tab renders `link.trace_id`/
 * `link.span_id` (truncated OTel identifiers, not secrets) and an
 * attribute COUNT badge — never a link attribute's actual value. There is
 * no code path here that could print a payload-derived string, so there is
 * nothing for a redactor to intercept. If a future change starts rendering
 * `link.attributes` values, add `redact` then — see
 * `SpanPanel.redaction-coverage.test.ts`, which enforces that every tab has
 * either a `redact` wiring or a written reason like this one.
 */
interface SpanLinksTabProps {
  span: VisualizationSpan
  onNavigateToTrace?: (traceId: string) => void
  onNavigateToSpan?: (spanId: string) => void
}

export function SpanLinksTab({
  span,
  onNavigateToTrace,
  onNavigateToSpan,
}: SpanLinksTabProps) {
  // Stamp each link with a stable position-derived id once. OTel allows
  // duplicate (trace_id, span_id) link pairs, so the index disambiguates
  // without re-deriving the key from array index inside render.
  const keyedLinks = useMemo(
    () =>
      (span.links || []).map((link, i) => ({
        link,
        id: `${link.trace_id}-${link.span_id}-${i}`,
      })),
    [span.links],
  )
  const links = keyedLinks

  if (links.length === 0) {
    return (
      <div className="p-4">
        <EmptyState
          icon={Link2}
          title="no linked traces"
          description="this span has no recorded span-to-span links"
        />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-2 p-4 font-inter">
      <div className="flex h-6 items-center gap-1.5">
        <span className="text-xs font-semibold text-foreground">
          Linked traces
        </span>
        <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
          {links.length}
        </span>
      </div>
      {links.map(({ link, id }) => {
        const isSameTrace = link.trace_id === span.trace_id
        const handleClick = () => {
          if (isSameTrace) {
            onNavigateToSpan?.(link.span_id)
          } else {
            onNavigateToTrace?.(link.trace_id)
          }
        }
        const attrCount = link.attributes
          ? Object.keys(link.attributes).length
          : 0
        return (
          <button
            key={id}
            type="button"
            onClick={handleClick}
            className="group/link flex w-full items-center gap-2.5 rounded-lg border border-border px-3 py-2 text-left transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            <ExternalLink
              className="size-4 shrink-0 text-muted-foreground transition-colors group-hover/link:text-foreground"
              aria-hidden
            />
            <div className="min-w-0 flex-1">
              <div className="truncate font-mono text-xs text-foreground tabular-nums">
                {link.trace_id.slice(0, 16)}…
              </div>
              <div className="font-mono text-[11px] text-muted-foreground tabular-nums">
                span {link.span_id.slice(0, 12)}
                {isSameTrace ? ' · this trace' : ''}
              </div>
            </div>
            {attrCount > 0 && (
              <span className="shrink-0 rounded-md bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground tabular-nums">
                +{attrCount} attrs
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}
