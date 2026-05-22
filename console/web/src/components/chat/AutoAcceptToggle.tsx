import { useId } from 'react'
import { cn } from '@/lib/utils'

interface AutoAcceptToggleProps {
  value: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
  className?: string
}

/**
 * Per-conversation toggle that flips the auto-accept-all-approvals
 * mode. Lives in the composer's bottom bar next to the mode picker.
 *
 * The label is rendered verbatim ("auto-accept: on" / "auto-accept:
 * off") rather than as an icon because this is a foot-gun: when ON,
 * every safe `agent_trigger` from the model is auto-resolved without a
 * human click (state-mutating, destructive, and egress calls remain
 * gated client-side by `auto-accept-policy.ts`). The user needs to
 * read the state at a glance.
 *
 * Accessibility:
 * - `role="switch"` + `aria-checked` carry the binary state to SRs;
 *   the visible "on"/"off" text matches the announced state so the
 *   "label in name" requirement is satisfied.
 * - `focus-visible` rings the control for keyboard navigation.
 * - The explanatory text lives in a visually-hidden node and is
 *   wired in via `aria-describedby` so keyboard/SR users get the
 *   same context that mouse users get from the `title` tooltip.
 * - `aria-disabled` rather than `disabled` so the control stays in
 *   the tab order and an SR can still read the current policy
 *   while streaming.
 *
 * Contrast: the ON state uses `bg-ink text-bg` (light theme:
 * near-black on near-white, ~16:1; dark theme: near-white on
 * near-black, similar) rather than `bg-accent text-bg`, which
 * measured ~3.2:1 on light bg and failed WCAG 1.4.3.
 */
export function AutoAcceptToggle({
  value,
  onChange,
  disabled,
  className,
}: AutoAcceptToggleProps) {
  const descId = useId()
  const interactive = !disabled

  const handleClick = () => {
    if (!interactive) return
    onChange(!value)
  }

  return (
    <>
      <button
        type="button"
        role="switch"
        aria-checked={value}
        aria-disabled={disabled || undefined}
        aria-describedby={descId}
        title={
          value
            ? 'auto-accept: ON. every safe approval prompt is resolved automatically (state-mutating calls still require a click). click to turn off.'
            : 'auto-accept: OFF. each approval prompt waits for your click. click to turn on.'
        }
        onClick={handleClick}
        className={cn(
          'font-mono text-[13px] px-3 py-1 border lowercase transition-colors',
          'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
          value
            ? 'border-ink bg-ink text-bg'
            : 'border-rule text-ink-faint hover:text-ink',
          disabled && 'opacity-50 cursor-not-allowed',
          className,
        )}
      >
        auto-accept: <span className="font-mono">{value ? 'on' : 'off'}</span>
      </button>
      <span
        id={descId}
        className="sr-only"
      >
        {value
          ? 'Auto-accept is on. Approval prompts for safe calls (reads, lookups, listings) are resolved automatically. Destructive or state-mutating calls still require a click.'
          : 'Auto-accept is off. Every approval prompt waits for an explicit click.'}
      </span>
    </>
  )
}
