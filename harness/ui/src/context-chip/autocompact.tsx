/**
 * The chip's auto-compact pieces (chat-projects §3.3.3, D170), apart from
 * the chip so they render in a test without the console: the label after
 * the counts and beside the hero's window, the mark over the stacked bar,
 * and the breakdown's line. Each renders nothing where the spec says
 * nothing: no label and no mark at the model's own limit.
 */

import { type AutocompactMark, chipLabel, compactsAfterTurn, meterLine } from '../lib/autocompact'

/** "· auto 160k" after the trigger's counts, or "· Compacts after this turn". */
export function ChipAutoLabel({
  total,
  mark,
  running,
}: {
  total: number
  mark: AutocompactMark | null
  running?: boolean
}) {
  const label = chipLabel(total, mark, running)
  if (!label) return null
  return (
    <span
      className="harness-ui-chip-auto"
      data-autocompact-label=""
      data-after-turn={compactsAfterTurn(total, mark?.effective ?? null, running) ? '' : undefined}
    >
      <span className="harness-ui-chip-sep" aria-hidden>
        ·
      </span>
      {label}
    </span>
  )
}

/** The same label beside the hero's "of 916k tokens". */
export function HeroAutoLabel({
  total,
  mark,
  running,
}: {
  total: number
  mark: AutocompactMark | null
  running?: boolean
}) {
  const label = chipLabel(total, mark, running)
  if (!label) return null
  return (
    <span
      className="harness-ui-hero-auto"
      data-autocompact-label=""
      data-after-turn={compactsAfterTurn(total, mark?.effective ?? null, running) ? '' : undefined}
    >
      {label}
    </span>
  )
}

/** The amber mark over the stacked bar, at `tick` (0–1). */
export function StackTick({ tick }: { tick: number }) {
  return (
    <span
      className="harness-ui-stack-tick"
      data-autocompact-tick=""
      aria-hidden
      style={{ left: `${Math.min(1, Math.max(0, tick)) * 100}%` }}
    />
  )
}

/** The breakdown's line: "Auto-compact at 160k · acme-web rule", or "At the
    model's limit". Nothing from a harness that writes no `autocompact`. */
export function AutocompactLine({ mark }: { mark: AutocompactMark | null }) {
  if (!mark) return null
  return (
    <p className="harness-ui-note" data-autocompact-line="">
      {meterLine(mark.effective, mark.rule)}
    </p>
  )
}
