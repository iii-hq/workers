import { Check, Copy, Package } from 'lucide-react'
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
    <div className="flex flex-col gap-2 p-4 font-inter">
      <div className="flex h-6 items-center gap-1.5">
        <Package className="size-4 text-muted-foreground" aria-hidden />
        <span className="text-xs font-semibold text-foreground">
          W3C baggage
        </span>
        <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
          {baggageEntries.length}
        </span>
      </div>

      <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
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
              className="group/bag w-full px-3 py-1.5 text-left transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="mb-0.5 font-mono text-[11px] text-muted-foreground">
                    {key}
                  </div>
                  <div className="font-mono text-xs break-all text-foreground">
                    {formatted}
                  </div>
                </div>
                {isCopied ? (
                  <Check
                    className="mt-0.5 size-4 shrink-0 text-ok"
                    aria-hidden
                  />
                ) : (
                  <Copy
                    className="mt-0.5 size-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/bag:opacity-100 group-focus-visible/bag:opacity-100"
                    aria-hidden
                  />
                )}
              </div>
            </button>
          )
        })}
      </div>
    </div>
  )
}
