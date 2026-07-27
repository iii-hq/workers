import { useState } from 'react'
import {
  formatUsageValue,
  hasReportedUsage,
  reportedValue,
  type TurnUsage,
} from '@/lib/session-usage'
import { formatTokenCount } from '@/lib/token-estimate'
import { cn } from '@/lib/utils'

/**
 * Per-turn usage, in the assistant message header beside the memory chip.
 *
 * Modeled on `MemoryChip`: same slot, same typography, same click-to-expand,
 * and the same "render nothing when there is nothing to say" guard — so a
 * session with no recorded usage looks exactly as it does today.
 *
 * The chip totals the turn; the expansion breaks it down per step. That
 * expansion is the point: it is where cache warmth becomes legible (step 0
 * cold, later steps warm), which no aggregate can show.
 */

interface TurnUsageChipProps {
  turn: TurnUsage
  /** Dock density — drops the cost figure to fit a narrow header. */
  compact?: boolean
}

export function TurnUsageChip({ turn, compact }: TurnUsageChipProps) {
  const [open, setOpen] = useState(false)
  const reported = hasReportedUsage(turn.totals)

  // Nothing measured and nothing in flight — stay invisible rather than
  // render a row of em dashes on every reply.
  if (!reported && !turn.streaming) return null

  const label = !reported
    ? '↑— ↓— · running'
    : [
        `↑${formatTokenCount(turn.totals.input)}`,
        `↓${formatTokenCount(turn.totals.output)}`,
        ...(compact || turn.totals.reported.cost === 0
          ? []
          : [`· ${formatUsageValue(turn.totals.costUsd, 'cost')}`]),
        ...(turn.streaming ? ['· running'] : []),
      ].join(' ')

  return (
    <span
      className="inline-flex flex-col items-start normal-case tracking-normal"
      data-turn-usage={turn.turnId}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={`what this turn consumed across ${turn.steps} model call${
          turn.steps === 1 ? '' : 's'
        } — click for the per-step breakdown`}
        className={cn(
          'font-mono text-[10px] lowercase px-1.5 py-0.5 border transition-colors tabular-nums',
          open
            ? 'border-accent text-ink'
            : 'border-rule text-ink-faint hover:border-ink hover:text-ink',
        )}
      >
        {label}
      </button>
      {open ? (
        <span className="mt-1 flex flex-col gap-1 border border-rule-2 bg-panel px-2 py-1.5 max-w-md overflow-x-auto">
          {turn.stepUsage.length === 0 ? (
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              no per-step usage recorded for this turn
            </span>
          ) : (
            turn.stepUsage.map(({ entryId, usage }, i) => (
              <span
                key={entryId}
                className="font-mono text-[11px] text-ink leading-snug tabular-nums whitespace-nowrap"
              >
                <span className="text-ink-ghost">step {i}</span>
                {'  ↑ '}
                {formatUsageValue(usage.input)}
                {'  ↓ '}
                {formatUsageValue(usage.output)}
                {typeof usage.cache_read === 'number' ? (
                  <span className="text-ink-faint">
                    {'  cache r '}
                    {formatUsageValue(usage.cache_read)}
                  </span>
                ) : null}
                {typeof usage.cost_usd === 'number' ? (
                  <span>{`  ${formatUsageValue(usage.cost_usd, 'cost')}`}</span>
                ) : null}
              </span>
            ))
          )}
          <span className="font-mono text-[10px] lowercase text-ink-ghost border-t border-rule-2 pt-1">
            turn total ↑{reportedValue(turn.totals, 'input', turn.totals.input)}{' '}
            ↓{reportedValue(turn.totals, 'output', turn.totals.output)} ·{' '}
            {reportedValue(turn.totals, 'cost', turn.totals.costUsd, 'cost')}
          </span>
        </span>
      ) : null}
    </span>
  )
}
