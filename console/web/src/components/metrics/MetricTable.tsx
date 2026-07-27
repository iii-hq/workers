import { cn } from '@/lib/utils'

/**
 * The metric table shared by the session metrics surfaces — a Tailwind port
 * of eval's aggregate table (`eval/ui/src/page/EvaluationDetail.tsx:399-454`)
 * in the console's own idiom. Eval's stylesheet is scoped under
 * `[data-iii-ui="eval"]` and unreachable from this SPA, so this only *looks*
 * like eval; nothing is shared but the visual language.
 *
 * Two columns rather than eval's four: there is no A/B comparison here, just
 * a metric, its value, and an optional muted note.
 */

export interface MetricRow {
  label: string
  /** Preformatted — callers use `formatUsageValue` / `reportedValue`. */
  value: string
  /** Muted right-hand annotation: a unit, a caveat, a derivation. */
  note?: string
  /**
   * Descriptive-only metric with no better/worse direction, marked with a
   * mid dot. Carries eval's `neutral` meaning: reading more cached tokens is
   * not "better", it means cost and latency reflect cache warmth.
   */
  neutral?: boolean
  tone?: 'default' | 'faint' | 'alert'
}

interface MetricTableProps {
  title: string
  /** One line under the title explaining where these numbers come from. */
  caption?: string
  rows: MetricRow[]
  className?: string
}

export function MetricTable({
  title,
  caption,
  rows,
  className,
}: MetricTableProps) {
  return (
    <section className={cn('flex flex-col', className)}>
      <h3 className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
        {title}
      </h3>
      {caption ? (
        <p className="mt-0.5 font-mono text-[11px] text-ink-ghost">{caption}</p>
      ) : null}
      <dl className="mt-2 divide-y divide-rule-2 border-y border-rule-2">
        {rows.map((row) => (
          <div
            key={row.label}
            className="flex items-baseline gap-3 py-1.5 font-mono text-[12px]"
          >
            <dt className="flex-1 min-w-0 truncate text-ink-faint">
              {row.label}
            </dt>
            {row.note ? (
              <span className="hidden sm:block flex-shrink-0 text-[11px] text-ink-ghost">
                {row.note}
              </span>
            ) : null}
            {row.neutral ? (
              <span
                className="flex-shrink-0 text-ink-ghost"
                title="descriptive only — no better or worse direction"
                aria-hidden
              >
                ·
              </span>
            ) : null}
            <dd
              className={cn(
                'flex-shrink-0 tabular-nums text-right min-w-[5.5rem]',
                row.tone === 'faint' && 'text-ink-faint',
                row.tone === 'alert' && 'text-alert',
                (!row.tone || row.tone === 'default') && 'text-ink',
              )}
            >
              {row.value}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  )
}
