// Square icon toggle button used by the waterfall and flame-graph
// toolbars. Filled with ink when `active`; outlined otherwise. The
// `label` argument doubles as `aria-label` and as the tooltip text.

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'

interface IconToggleButtonProps {
  active: boolean
  onClick: () => void
  label: string
  children: React.ReactNode
}

export function IconToggleButton({
  active,
  onClick,
  label,
  children,
}: IconToggleButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          onClick={onClick}
          aria-label={label}
          aria-pressed={active}
          className={cn(
            'inline-flex items-center justify-center w-7 h-7 border transition-colors',
            active
              ? 'bg-ink text-bg border-ink'
              : 'bg-bg text-ink-faint border-rule hover:text-ink hover:border-ink',
          )}
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  )
}
