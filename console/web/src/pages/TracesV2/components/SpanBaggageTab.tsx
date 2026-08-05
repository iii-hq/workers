import { Copy, Package } from 'lucide-react'
import { useMemo } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
import type { VisualizationSpan } from '../lib/traceTransform'
import { useCopyToClipboard } from '../lib/traceUtils'

/**
 * No `redact` prop, deliberately: sandbox-code-runner never touches the OTel
 * baggage API (`grep -ri baggage sandbox-code-runner/src` — zero hits in the
 * crate). Baggage is CALLER-set routing/identity context copied onto every
 * span in its scope (iii-helpers'
 * `BaggageSpanProcessor` — turn identity, trace tags, `iii.function.id`),
 * propagated top-down from the engine/harness; a worker's own internal
 * capability (sandbox-code-runner's `runtime_id`) never flows the
 * other way into it. If a future worker ever starts stamping baggage,
 * revisit this — see `SpanPanel.redaction-coverage.test.ts`, which enforces
 * that every tab has either a `redact` wiring or a written reason like
 * this one.
 */
interface SpanBaggageTabProps {
  span: VisualizationSpan
}

export function SpanBaggageTab({ span }: SpanBaggageTabProps) {
  const { copiedKey, copy } = useCopyToClipboard()

  const baggageEntries = useMemo(
    () =>
      Object.entries(span.attributes || {})
        .filter(([key]) => key.startsWith('baggage.'))
        .map(
          ([key, value]) =>
            [key.replace('baggage.', ''), value] as [string, unknown],
        ),
    [span.attributes],
  )

  if (baggageEntries.length === 0) {
    return (
      <div className="p-4">
        <EmptyState
          icon={Package}
          title="no baggage context"
          description="w3c baggage propagates key-value pairs across worker boundaries. none was attached to this span."
        />
      </div>
    )
  }

  return (
    <div className="p-5 space-y-3">
      <div className="flex items-center gap-2 px-1">
        <Package className="w-3.5 h-3.5 text-accent" />
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          w3c baggage context
        </span>
      </div>

      <div className="border border-rule bg-bg divide-y divide-rule">
        {baggageEntries.map(([key, value]) => {
          const formatted =
            typeof value === 'object'
              ? JSON.stringify(value, null, 2)
              : String(value)
          const isCopied = copiedKey === key
          return (
            <button
              key={key}
              type="button"
              onClick={() => copy(key, `${key}: ${formatted}`)}
              aria-label={`copy ${key} to clipboard`}
              className="w-full px-4 py-2.5 hover:bg-panel transition-colors text-left group"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="flex-1 min-w-0">
                  <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-0.5">
                    {key}
                  </div>
                  <div className="font-mono text-[12px] text-ink break-all">
                    {formatted}
                  </div>
                </div>
                {isCopied ? (
                  <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-accent flex-shrink-0 mt-0.5">
                    copied
                  </span>
                ) : (
                  <Copy className="w-3 h-3 text-ink-ghost opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0 mt-0.5" />
                )}
              </div>
            </button>
          )
        })}
      </div>

      <div className="font-mono text-[10px] text-ink-ghost text-center pt-2 lowercase tabular-nums">
        {baggageEntries.length} baggage item
        {baggageEntries.length !== 1 ? 's' : ''}
      </div>
    </div>
  )
}
