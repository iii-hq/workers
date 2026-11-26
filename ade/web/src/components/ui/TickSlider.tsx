import { Slider as SliderPrimitive } from '@base-ui/react/slider'
import type { CSSProperties } from 'react'
import { cn } from '@/lib/utils'

export interface SliderTick {
  value: number
  /** What the mark stands for (a model's name): its tooltip. */
  label: string
}

export interface TickSliderProps {
  value: number
  min: number
  max: number
  step?: number
  /** Grey marks on the track; values outside min–max are left out. */
  ticks?: readonly SliderTick[]
  onValueChange: (value: number) => void
  /** The drag or the key press is over: the place to write. */
  onValueCommitted?: (value: number) => void
  disabled?: boolean
  /** The thumb's accessible name. */
  label: string
  /** What a screen reader says for a value ("400k"). */
  valueText?: (value: number) => string
  className?: string
}

type TickStyle = CSSProperties & { '--tick-x': string }

/**
 * A one-thumb slider with grey ticks on its track: the Base UI slider in
 * the shadcn look, the thumb centred on its value so a tick at a value sits
 * under the thumb there (chat-projects §3.3.1: a tick per model's limit).
 */
export function TickSlider({
  value,
  min,
  max,
  step = 1,
  ticks = [],
  onValueChange,
  onValueCommitted,
  disabled,
  label,
  valueText,
  className,
}: TickSliderProps) {
  const span = max - min
  const shown = ticks.filter((t) => t.value >= min && t.value <= max)
  return (
    <SliderPrimitive.Root
      data-slot="tick-slider"
      value={value}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      thumbAlignment="center"
      onValueChange={(next) => onValueChange(next as number)}
      onValueCommitted={(next) => onValueCommitted?.(next as number)}
      className={cn('w-full', className)}
    >
      <SliderPrimitive.Control className="relative flex h-6 w-full touch-none items-center select-none data-disabled:opacity-50">
        <SliderPrimitive.Track className="relative h-1 w-full grow rounded-full bg-muted select-none">
          <SliderPrimitive.Indicator className="rounded-full bg-primary select-none data-horizontal:h-full" />
          {shown.map((tick) => (
            <span
              key={`${tick.value}:${tick.label}`}
              data-slider-tick={tick.value}
              title={tick.label}
              aria-hidden
              className="absolute top-1/2 left-(--tick-x) h-3 w-px -translate-x-1/2 -translate-y-1/2 rounded-full bg-ink-ghost"
              style={
                {
                  '--tick-x': `${span > 0 ? ((tick.value - min) / span) * 100 : 0}%`,
                } as TickStyle
              }
            />
          ))}
        </SliderPrimitive.Track>
        <SliderPrimitive.Thumb
          getAriaLabel={() => label}
          getAriaValueText={
            valueText ? (_formatted, v) => valueText(v) : undefined
          }
          className="relative block size-4 shrink-0 rounded-full border border-ring bg-background ring-ring/50 transition-[color,box-shadow] select-none after:absolute after:-inset-2 hover:ring-3 focus-visible:ring-3 focus-visible:outline-hidden active:ring-3 disabled:pointer-events-none disabled:opacity-50 pointer-coarse:size-5"
        />
      </SliderPrimitive.Control>
    </SliderPrimitive.Root>
  )
}
