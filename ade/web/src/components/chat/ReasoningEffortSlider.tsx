import {
  type CSSProperties,
  type RefObject,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { cn } from '@/lib/utils'
import type { ReasoningEffortOption, ThinkingLevel } from '@/types/chat'
import { formatEffortLabel } from './model-picker-presentation'
import './ReasoningEffortSlider.css'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/** Position of a level along the slider, 0 at the first stop and 1 at the last. */
export function effortRatio(index: number, count: number): number {
  if (count <= 1 || index <= 0) return 0
  return Math.min(1, index / (count - 1))
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

/**
 * Text states swap (transitions.dev #04): when `next` changes, the label on
 * screen exits up through a blur, then the new text enters from below. The
 * returned `text` therefore lags `next` by one `--text-swap-dur`; rapid
 * changes restart the exit and land on the last value.
 */
function useTextSwap(next: string): {
  ref: RefObject<HTMLSpanElement | null>
  text: string
} {
  const ref = useRef<HTMLSpanElement>(null)
  const [shown, setShown] = useState(next)
  const enteringRef = useRef(false)

  useEffect(() => {
    const el = ref.current
    if (shown === next) {
      // A change that came back to the visible text before the exit landed.
      el?.classList.remove('is-exit')
      return
    }
    if (!el || prefersReducedMotion()) {
      setShown(next)
      return
    }
    el.classList.add('is-exit')
    const duration =
      Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue(
          '--text-swap-dur',
        ),
      ) || 150
    const timer = window.setTimeout(() => {
      enteringRef.current = true
      setShown(next)
    }, duration)
    return () => window.clearTimeout(timer)
  }, [next, shown])

  // biome-ignore lint/correctness/useExhaustiveDependencies: `shown` is the trigger — the enter phase must run in the same commit that swapped the text
  useLayoutEffect(() => {
    const el = ref.current
    if (!el || !enteringRef.current) return
    enteringRef.current = false
    el.classList.remove('is-exit')
    el.classList.add('is-enter-start')
    void el.offsetHeight // reflow so removing the class below transitions
    el.classList.remove('is-enter-start')
  }, [shown])

  return { ref, text: shown }
}

/** A level's name as the slider spells it: `Default`, `Off`, `High`. */
export function effortStopLabel(effort: string): string {
  if (effort === 'default') return 'Default'
  return formatEffortLabel(effort) || effort
}

interface ReasoningEffortSliderProps {
  /** Levels in ascending order — `default` first, the deepest effort last. */
  options: ReasoningEffortOption[]
  value: ThinkingLevel
  onChange: (next: ThinkingLevel) => void
  disabled?: boolean
  className?: string
  /**
   * `auto` puts label, track and value on one row from 640px up (a wide
   * sheet page); `stacked` keeps the track on its own full-width row at
   * any width — the 360px menu, where the stop names need the room.
   */
  layout?: 'auto' | 'stacked'
  /** Name every stop under the track (click one to jump to it). Defaults on. */
  showStops?: boolean
}

/**
 * Reasoning effort as a pill slider: drag, click a dot, or arrow the thumb
 * across the levels. The fill and the level name take on more of the accent
 * the further right the level sits, so the scale reads at a glance; the
 * level's description is the tooltip on the thumb and the name. Only the
 * levels the model offers are stops, so the scale never implies a level
 * the model lacks; each stop is named under the track, the current one in
 * the page ink.
 */
export function ReasoningEffortSlider({
  options,
  value,
  onChange,
  disabled,
  className,
  layout = 'auto',
  showStops = true,
}: ReasoningEffortSliderProps) {
  const id = useId()
  const foundIndex = options.findIndex((option) => option.effort === value)
  const index = foundIndex >= 0 ? foundIndex : 0
  const current = options[index]
  const ratio = effortRatio(index, options.length)
  const swap = useTextSwap(current?.effort ?? '')

  if (!current) return null

  return (
    <div
      className={cn(
        'reasoning-effort',
        layout === 'stacked' && 'reasoning-effort--stacked',
        showStops && 'reasoning-effort--with-stops',
        disabled && 'pointer-events-none opacity-40',
        className,
      )}
      style={
        {
          '--effort-ratio': ratio,
          '--effort-stops': options.length,
        } as CSSProperties
      }
    >
      <label
        htmlFor={id}
        className="reasoning-effort__label truncate font-sans text-sm font-medium text-ink sm:text-[13px]"
      >
        Reasoning effort
      </label>
      <div className="reasoning-effort__track">
        <span className="reasoning-effort__fill" aria-hidden />
        {options.map((option, optionIndex) => (
          <span
            key={option.effort}
            aria-hidden
            className={cn(
              'reasoning-effort__dot',
              option.disabled && 'reasoning-effort__dot--disabled',
            )}
            title={option.disabled ? option.description : undefined}
            style={
              {
                '--effort-stop': effortRatio(optionIndex, options.length),
              } as CSSProperties
            }
          />
        ))}
        <input
          id={id}
          type="range"
          min={0}
          max={Math.max(0, options.length - 1)}
          step={1}
          value={index}
          aria-valuetext={current.effort}
          title={current.description}
          disabled={disabled}
          onChange={(event) => {
            let at = Number(event.target.value)
            // A disabled stop (Off the catalog could not confirm) is skipped
            // in the direction of travel, so the thumb never rests on it.
            const step = at >= index ? 1 : -1
            while (options[at]?.disabled) at += step
            const next = options[at]
            if (next && next.effort !== value) onChange(next.effort)
          }}
          className="reasoning-effort__range"
        />
      </div>
      {/* Every level name is stacked invisibly under the visible one so the
          cell keeps the widest label's width and the track never resizes. */}
      <span
        className="reasoning-effort__value grid font-sans text-sm font-semibold capitalize sm:text-[13px]"
        title={current.description}
      >
        {options.map((option) => (
          <span
            key={option.effort}
            aria-hidden
            className="invisible col-start-1 row-start-1 text-right"
          >
            {option.effort}
          </span>
        ))}
        <span
          ref={swap.ref}
          data-effort-label
          className="t-text-swap col-start-1 row-start-1 text-right"
        >
          {swap.text}
        </span>
      </span>
      {showStops ? (
        <div className="reasoning-effort__stops" data-effort-stops>
          {options.map((option, optionIndex) => {
            const current = optionIndex === index
            const name = effortStopLabel(option.effort)
            return (
              <button
                key={option.effort}
                type="button"
                // The range is the one keyboard control; a stop name is a
                // pointer shortcut to its level and stays out of the tab
                // order so Tab never walks seven near-duplicates.
                tabIndex={-1}
                aria-label={`${name} reasoning effort`}
                aria-pressed={current}
                disabled={disabled || option.disabled}
                title={option.description}
                onClick={() => {
                  if (option.effort !== value) onChange(option.effort)
                }}
                className={cn(
                  'reasoning-effort__stop font-sans text-xs',
                  current
                    ? 'font-medium text-ink'
                    : 'text-ink-faint hover:text-ink',
                  option.disabled && 'reasoning-effort__stop--disabled',
                )}
                style={
                  {
                    '--effort-stop': effortRatio(optionIndex, options.length),
                  } as CSSProperties
                }
              >
                {name}
              </button>
            )
          })}
        </div>
      ) : null}
    </div>
  )
}
