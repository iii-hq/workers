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
 * every `agent_trigger` from the model is auto-resolved without a
 * human click. The user needs to read the state at a glance.
 *
 * The ON style uses the `accent` colour against `bg-ink` to visually
 * separate this toggle from the segmented `ModePicker` next to it —
 * the two controls own different concepts (mode vs. approval policy)
 * and shouldn't look interchangeable.
 */
export function AutoAcceptToggle({
  value,
  onChange,
  disabled,
  className,
}: AutoAcceptToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={value}
      aria-label={
        value
          ? 'Auto-accept is on — disable it'
          : 'Auto-accept is off — enable it'
      }
      title={
        value
          ? 'auto-accept: ON. every approval prompt is resolved automatically. click to turn off.'
          : 'auto-accept: OFF. each approval prompt waits for your click. click to turn on.'
      }
      disabled={disabled}
      onClick={() => onChange(!value)}
      className={cn(
        'font-mono text-[13px] px-3 py-1 border lowercase transition-colors',
        value
          ? 'border-accent bg-accent text-bg'
          : 'border-rule text-ink-faint hover:text-ink',
        disabled && 'opacity-50 cursor-not-allowed',
        className,
      )}
    >
      auto-accept: <strong className="font-mono">{value ? 'on' : 'off'}</strong>
    </button>
  )
}
