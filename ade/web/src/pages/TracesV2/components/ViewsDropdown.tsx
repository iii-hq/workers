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
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
} from '@/components/shadcn/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Input } from '@/components/shadcn/input'
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
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant={activeView ? 'secondary' : 'outline'}
              className="rounded-xl"
              aria-label="switch traces view"
            />
          }
        >
          <SlidersVertical data-icon="inline-start" />
          <span className="max-w-[160px] truncate">{triggerLabel}</span>
          {activeModified && (
            <Circle
              className="size-1.5! fill-amber-500 text-amber-500"
              aria-label="view modified"
            />
          )}
          <ChevronDown data-icon="inline-end" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuLabel>Views</DropdownMenuLabel>
          <DropdownMenuItem onClick={() => onSelectView(null)}>
            <span className="w-3">
              {!activeView && <Check className="size-4" />}
            </span>
            all traces
          </DropdownMenuItem>
          {views.map((view) => (
            <DropdownMenuItem key={view.id} onClick={() => onSelectView(view)}>
              <span className="w-3">
                {view.id === activeViewId && <Check className="size-4" />}
              </span>
              <span className="truncate">{view.name}</span>
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => openDialog('save')}>
            save current as view…
          </DropdownMenuItem>
          {activeView && (
            <>
              <DropdownMenuItem
                disabled={!activeModified}
                onClick={onUpdateActive}
              >
                update “{activeView.name}”
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => openDialog('rename')}>
                rename…
              </DropdownMenuItem>
              <DropdownMenuItem onClick={onDeleteActive}>
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
        <DialogContent>
          <DialogTitle>
            {nameDialog === 'rename' ? 'Rename view' : 'Save view'}
          </DialogTitle>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              submitName()
            }}
            className="flex flex-col gap-3 mt-3"
          >
            <Input
              autoFocus
              type="text"
              value={nameInput}
              onChange={(e) => setNameInput(e.target.value)}
              placeholder="View name"
            />
            <div className="flex items-center justify-end gap-2">
              <DialogClose
                render={<Button variant="ghost" size="sm" type="button" />}
              >
                Cancel
              </DialogClose>
              <Button size="sm" type="submit" disabled={!nameInput.trim()}>
                {nameDialog === 'rename' ? 'Rename' : 'Save'}
              </Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
