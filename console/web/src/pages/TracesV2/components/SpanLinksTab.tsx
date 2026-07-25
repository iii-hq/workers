import { ExternalLink, Link2 } from 'lucide-react'
import { useMemo } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
import type { VisualizationSpan } from '../lib/traceTransform'

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
    <div className="p-3 space-y-2">
      <div className="font-mono text-[11px] text-ink-faint uppercase tracking-[0.06em] px-1 mb-2">
        linked traces ({links.length})
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
            className="w-full flex items-center gap-2 px-3 py-2 rounded-sm bg-surface hover:bg-surface-hover transition-colors text-left group"
          >
            <ExternalLink className="w-3 h-3 text-ink-faint group-hover:text-accent flex-shrink-0" />
            <div className="flex-1 min-w-0">
              <div className="font-mono text-[11px] text-ink-faint truncate group-hover:text-ink tabular-nums lowercase">
                {link.trace_id.slice(0, 16)}…
              </div>
              <div className="font-mono text-[10px] text-ink-ghost tabular-nums lowercase">
                span: {link.span_id.slice(0, 12)}
              </div>
            </div>
            {attrCount > 0 && (
              <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint rounded-xs bg-paper-2 px-1.5 py-0.5 flex-shrink-0 tabular-nums">
                +{attrCount} attrs
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}
