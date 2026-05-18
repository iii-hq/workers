import { ArrowDown, Zap } from 'lucide-react'
import { useMemo } from 'react'
import { cn } from '@/lib/utils'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import { classifySpanType, formatDuration } from '../lib/traceUtils'

interface WorkflowChainProps {
  data: WaterfallData
  onSpanClick?: (span: VisualizationSpan) => void
}

interface ChainStep {
  span: VisualizationSpan
  type: 'trigger' | 'enqueue' | 'function'
  label: string
  sublabel?: string
}

// Schematic palette doesn't support chromatic groupings, so the three
// chain types share the same bordered cell chrome. Tone is conveyed via
// an accent column (border-l-2) and the type label text color:
//   - trigger: accent (orange in light, electric-blue in dark)
//   - enqueue: warn
//   - function: ink-faint (no accent column)
const TYPE_TONE: Record<
  ChainStep['type'],
  { accentBar: string; dot: string; text: string }
> = {
  trigger: {
    accentBar: 'border-l-accent',
    dot: 'bg-accent',
    text: 'text-accent',
  },
  enqueue: {
    accentBar: 'border-l-warn',
    dot: 'bg-warn',
    text: 'text-warn',
  },
  function: {
    accentBar: 'border-l-rule',
    dot: 'bg-ink',
    text: 'text-ink',
  },
}

export function WorkflowChain({ data, onSpanClick }: WorkflowChainProps) {
  const chain = useMemo(() => {
    const steps: ChainStep[] = []

    for (const span of data.spans) {
      const attrs = span.attributes || {}
      const topic = attrs['messaging.destination.name'] as string | undefined
      const functionId = (attrs['faas.invoked_name'] || attrs.function_id) as
        | string
        | undefined
      const spanType = classifySpanType(span)

      if (spanType === 'enqueue') {
        steps.push({
          span,
          type: 'enqueue',
          label: topic || 'enqueue',
          sublabel: functionId,
        })
      } else if (spanType === 'trigger') {
        steps.push({
          span,
          type: 'trigger',
          label: functionId || span.name,
          sublabel: formatDuration(span.duration_ms),
        })
      } else if (functionId && span.depth === 0) {
        steps.push({
          span,
          type: 'function',
          label: functionId,
          sublabel: formatDuration(span.duration_ms),
        })
      }
    }

    return steps
  }, [data.spans])

  if (chain.length < 2) return null

  return (
    <div className="px-4 py-3 border-t border-rule-2">
      <div className="text-[10px] text-ink-faint uppercase tracking-[0.06em] mb-2 flex items-center gap-1.5 lowercase">
        <Zap className="w-3 h-3" />
        <span className="uppercase tracking-[0.06em]">workflow chain</span>
      </div>
      <div className="space-y-0">
        {chain.map((step, i) => {
          const tone = TYPE_TONE[step.type]
          return (
            <div key={step.span.span_id}>
              <button
                type="button"
                onClick={() => onSpanClick?.(step.span)}
                className={cn(
                  'w-full flex items-center gap-2 px-2.5 py-1.5 bg-bg border border-rule border-l-2 hover:bg-panel transition-colors text-left group',
                  tone.accentBar,
                )}
              >
                <span
                  aria-hidden
                  className={cn(
                    'w-1.5 h-1.5 rounded-full flex-shrink-0',
                    tone.dot,
                  )}
                />
                <div className="flex-1 min-w-0">
                  <div
                    className={cn(
                      'text-[11px] font-mono truncate lowercase',
                      tone.text,
                    )}
                  >
                    {step.label}
                  </div>
                  {step.sublabel && (
                    <div className="text-[9px] text-ink-ghost truncate font-mono tabular-nums lowercase">
                      {step.sublabel}
                    </div>
                  )}
                </div>
                <span className="text-[9px] font-mono text-ink-ghost flex-shrink-0 uppercase tracking-[0.06em]">
                  {step.type}
                </span>
              </button>
              {i < chain.length - 1 && (
                <div className="flex justify-center py-0.5">
                  <ArrowDown className="w-3 h-3 text-ink-ghost" />
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
