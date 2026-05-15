import { cn } from '@/lib/utils'

interface WordmarkProps {
  className?: string
}

/**
 * The "iii" wordmark — three lowercase "i"s as identical square units. Per §6
 * of DESIGN.md the mark is six rectangles (stem + tittle per "i"), all in ink,
 * sharing the same unit. No curves, no color, deliberate negative space.
 */
export function Wordmark({ className }: WordmarkProps) {
  return (
    <span
      role="img"
      aria-label="iii"
      className={cn(
        'inline-flex items-end gap-[6px] font-mono text-ink',
        className,
      )}
    >
      <Glyph />
      <Glyph />
      <Glyph />
    </span>
  )
}

function Glyph() {
  return (
    <span aria-hidden className="inline-flex flex-col items-center gap-[3px]">
      {/* tittle */}
      <span className="block size-[4px] bg-ink" />
      {/* stem */}
      <span className="block w-[4px] h-[12px] bg-ink" />
    </span>
  )
}
