import { Plus, X } from 'lucide-react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import type { TabScreen } from '@/lib/workspace-tabs'
import type { ScreenOption } from './use-screen-options'

interface EmptyPaneProps {
  screenOptions: ScreenOption[]
  onAttach: (screen: TabScreen) => void
  /** Present when the column can be dropped (multi-column tabs only). */
  onRemove?: () => void
}

/**
 * An empty workspace-tab column: nothing attached yet. The affordances are
 * the attach dropdown listing every available screen (chat + pages +
 * injected pages) and — in a split — removing the column again. Generous
 * padding keeps both usable on narrow/mobile panes.
 */
export function EmptyPane({
  screenOptions,
  onAttach,
  onRemove,
}: EmptyPaneProps) {
  return (
    <div className="flex-1 flex flex-col items-center justify-center gap-3 p-6 text-center">
      <div className="font-mono text-[12px] lowercase text-ink-ghost">
        nothing attached to this panel yet
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="inline-flex items-center gap-2 h-9 px-4 rounded-sm bg-surface font-mono text-[13px] lowercase text-ink hover:bg-surface-hover data-[state=open]:bg-surface-hover transition-colors"
          >
            <Plus className="size-3.5" />
            attach a page
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="center" sideOffset={6}>
          {screenOptions.map((option) => (
            <DropdownMenuItem
              key={option.value}
              onSelect={() => onAttach(option.value)}
            >
              {option.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      {onRemove ? (
        <button
          type="button"
          onClick={onRemove}
          className="inline-flex items-center gap-1.5 h-7 px-2 rounded-sm font-mono text-[11px] lowercase text-ink-ghost hover:text-ink hover:bg-surface-hover transition-colors"
        >
          <X className="size-3" />
          remove this panel
        </button>
      ) : null}
    </div>
  )
}
