import { type LucideIcon, Monitor, Moon, Sun } from 'lucide-react'
import type { KeyboardEvent } from 'react'
import type { ThemePreference } from '@/hooks/use-theme'
import { cn } from '@/lib/utils'

const OPTIONS: ReadonlyArray<{
  value: ThemePreference
  label: string
  Icon: LucideIcon
}> = [
  { value: 'light', label: 'Light theme', Icon: Sun },
  { value: 'system', label: 'Follow the system theme', Icon: Monitor },
  { value: 'dark', label: 'Dark theme', Icon: Moon },
]

/** One segment, in pixels: the pill is 32px tall with a 2px inset. */
const SEGMENT = 28

interface ThemeSwitchProps {
  value: ThemePreference
  onChange: (next: ThemePreference) => void
  className?: string
}

/**
 * The theme as a three-way pill — sun, monitor, moon — with one raised
 * disc that slides to the chosen segment. Stroke glyphs, like every other
 * icon in the chrome. A radio group to assistive tech: arrow keys move the
 * choice, and each segment names the theme it picks.
 */
export function ThemeSwitch({ value, onChange, className }: ThemeSwitchProps) {
  const index = Math.max(
    0,
    OPTIONS.findIndex((option) => option.value === value),
  )

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step =
      event.key === 'ArrowRight' || event.key === 'ArrowDown'
        ? 1
        : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
          ? -1
          : 0
    if (step === 0) return
    event.preventDefault()
    const next = OPTIONS[(index + step + OPTIONS.length) % OPTIONS.length]
    onChange(next.value)
    event.currentTarget
      .querySelector<HTMLButtonElement>(`[data-value="${next.value}"]`)
      ?.focus()
  }

  return (
    <div
      role="radiogroup"
      aria-label="Theme"
      onKeyDown={onKeyDown}
      className={cn(
        'relative inline-flex h-8 shrink-0 items-center rounded-full bg-neutral-300/60 p-0.5 dark:bg-neutral-800',
        className,
      )}
    >
      {/* The disc: one element that slides, so the change reads as a move
          rather than two segments swapping. */}
      <span
        aria-hidden
        className="absolute top-0.5 left-0.5 size-7 rounded-full bg-white shadow-sm transition-transform duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] dark:bg-neutral-600"
        style={{ transform: `translateX(${index * SEGMENT}px)` }}
      />
      {OPTIONS.map(({ value: option, label, Icon }) => {
        const checked = option === value
        return (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={checked}
            aria-label={label}
            title={label}
            data-value={option}
            tabIndex={checked ? 0 : -1}
            onClick={() => onChange(option)}
            className={cn(
              'relative z-10 flex size-7 items-center justify-center rounded-full transition-colors duration-[var(--motion-duration-control)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:focus-visible:ring-neutral-400',
              checked
                ? 'text-neutral-900 dark:text-neutral-50'
                : 'text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-200',
            )}
          >
            <Icon className="size-4" strokeWidth={1.75} aria-hidden />
          </button>
        )
      })}
    </div>
  )
}
