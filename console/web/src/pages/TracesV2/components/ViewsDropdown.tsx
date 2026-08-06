// Saved-view switcher for the TRACES tab.
//
// One dropdown carries the whole lifecycle: switch between "all traces" and
// the named views, save the current state as a new view, push local changes
// into the active view ("update"), rename, delete. Views persist in the
// server-side `console` configuration entry (see hooks/useTraceViews); the
// component renders nothing when that entry is unreachable so the tab
// degrades to plain filters.

import { Check, ChevronDown, Circle, SlidersVertical } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
} from '@/components/ui/Dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { cn } from '@/lib/utils'
import type { TracesView } from '../lib/tracesViews'

interface ViewsDropdownProps {
  views: TracesView[]
  activeViewId: string | null
  /** Live state diverges from the active view's snapshot. */
  activeModified: boolean
  onSelectView: (view: TracesView | null) => void
  onSaveNew: (name: string) => void
  onUpdateActive: () => void
  onRenameActive: (name: string) => void
  onDeleteActive: () => void
}

type NameDialogMode = 'save' | 'rename' | null

export function ViewsDropdown({
  views,
  activeViewId,
  activeModified,
  onSelectView,
  onSaveNew,
  onUpdateActive,
  onRenameActive,
  onDeleteActive,
}: ViewsDropdownProps) {
  const [nameDialog, setNameDialog] = useState<NameDialogMode>(null)
  const [nameInput, setNameInput] = useState('')

  const activeView = views.find((v) => v.id === activeViewId) ?? null
  const triggerLabel = activeView ? activeView.name : 'all traces'

  const openDialog = (mode: Exclude<NameDialogMode, null>) => {
    setNameInput(mode === 'rename' ? (activeView?.name ?? '') : '')
    setNameDialog(mode)
  }

  const submitName = () => {
    const name = nameInput.trim()
    if (!name) return
    if (nameDialog === 'save') onSaveNew(name)
    else if (nameDialog === 'rename') onRenameActive(name)
    setNameDialog(null)
  }

  return (
    <>
      {/* Non-modal: the save/rename Dialog opens from a menu item, and two
          modal Radix layers racing over the body pointer-events lock leave
          the page frozen after the dialog closes (radix-ui/primitives#1836).
          The dialog must stay the only modal layer. */}
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className={cn(
              'inline-flex items-center gap-2 h-8 px-2.5 rounded-sm font-mono text-[12px] lowercase transition-colors',
              activeView
                ? 'bg-accent-muted text-ink'
                : 'bg-surface text-ink-faint hover:text-ink hover:bg-surface-hover',
            )}
            aria-label="switch traces view"
          >
            <SlidersVertical className="w-3 h-3" />
            <span className="max-w-[160px] truncate">{triggerLabel}</span>
            {activeModified && (
              <Circle
                className="w-1.5 h-1.5 fill-warn text-warn"
                aria-label="view modified"
              />
            )}
            <ChevronDown className="w-3 h-3" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuLabel>views</DropdownMenuLabel>
          <DropdownMenuItem onSelect={() => onSelectView(null)}>
            <span className="w-3">
              {!activeView && <Check className="w-3 h-3" />}
            </span>
            all traces
          </DropdownMenuItem>
          {views.map((view) => (
            <DropdownMenuItem key={view.id} onSelect={() => onSelectView(view)}>
              <span className="w-3">
                {view.id === activeViewId && <Check className="w-3 h-3" />}
              </span>
              <span className="truncate">{view.name}</span>
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => openDialog('save')}>
            save current as view…
          </DropdownMenuItem>
          {activeView && (
            <>
              <DropdownMenuItem
                disabled={!activeModified}
                onSelect={onUpdateActive}
              >
                update “{activeView.name}”
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => openDialog('rename')}>
                rename…
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={onDeleteActive}>
                delete view
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>

      <Dialog
        open={nameDialog !== null}
        onOpenChange={(open) => !open && setNameDialog(null)}
      >
        <DialogContent className="max-w-sm">
          <DialogTitle>
            {nameDialog === 'rename' ? 'rename view' : 'save view'}
          </DialogTitle>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              submitName()
            }}
            className="flex flex-col gap-3 mt-3"
          >
            <input
              autoFocus
              type="text"
              value={nameInput}
              onChange={(e) => setNameInput(e.target.value)}
              placeholder="view name"
              className="h-8 px-2 font-mono text-[12px] rounded-sm bg-surface border border-transparent text-ink placeholder:text-ink-ghost hover:bg-surface-hover focus:outline-none focus:border-rule-focus transition-colors"
            />
            <div className="flex items-center justify-end gap-2">
              <DialogClose asChild>
                <Button variant="ghost" size="sm" type="button">
                  cancel
                </Button>
              </DialogClose>
              <Button
                variant="primary"
                size="sm"
                type="submit"
                disabled={!nameInput.trim()}
              >
                {nameDialog === 'rename' ? 'rename' : 'save'}
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
