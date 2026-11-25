import { cn } from 'cn'
import { Lock } from 'lucide-react'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/shadcn/popover'

/**
 * A side chat's model, in the composer where the model picker would be
 * (chat-projects §2.3.2): a lock, "Claude Opus 5.5 · High", no chevron. A
 * tap says why it cannot change instead of opening a picker.
 */
export function LockedModelLabel({
  label,
  hint,
  className,
}: {
  label: string
  hint: string
  className?: string
}) {
  return (
    <Popover>
      <PopoverTrigger
        data-locked-model=""
        aria-label={`${label}, locked`}
        className={cn(
          'flex h-8 min-w-0 items-center gap-1.5 rounded-lg px-2 font-inter text-xs text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 pointer-coarse:h-11',
          className,
        )}
      >
        <Lock aria-hidden className="size-4 shrink-0" />
        <span className="min-w-0 truncate">{label}</span>
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="start"
        data-locked-model-hint=""
        className="w-64 text-sm"
      >
        {hint}
      </PopoverContent>
    </Popover>
  )
}
