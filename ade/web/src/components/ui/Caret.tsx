import { cn } from '@/lib/utils'

interface CaretProps {
  className?: string
}

/**
 * The streaming caret: a rounded bar that breathes while the model is
 * generating (`stream-caret`, index.css). Sized to the surrounding text's
 * x-height so it reads as a cursor, not a block.
 */
export function Caret({ className }: CaretProps) {
  return (
    <span
      aria-hidden
      className={cn(
        'stream-caret inline-block h-[14px] w-[7px] rounded-[2px] bg-ink align-middle',
        className,
      )}
    />
  )
}
