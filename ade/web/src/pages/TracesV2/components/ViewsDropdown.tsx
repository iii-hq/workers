// Saved-view switcher for the TRACES tab.
//
// One dropdown carries the whole lifecycle: switch between "all traces" and
// the named views, save the current state as a new view, push local changes
// into the active view ("update"), rename, delete. Views persist in the
// server-side `console` configuration entry (see hooks/useTraceViews); the
// component renders nothing when that entry is unreachable so the tab
// degrades to plain filters.

import { ChevronDown, Circle, SlidersVertical } from 'lucide-react'
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
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Input } from '@/components/shadcn/input'
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
  const triggerLabel = activeView ? activeView.name : 'All traces'

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
            // A quiet chip at the start of the row: ghost on "All traces",
            // the soft fill once a saved view is active.
            <Button
              type="button"
              variant={activeView ? 'secondary' : 'ghost'}
              size="sm"
              className={cn(
                'max-w-[14rem] rounded-md',
                !activeView && 'text-muted-foreground',
              )}
              aria-label="switch traces view"
            />
          }
        >
          <SlidersVertical data-icon="inline-start" className="size-4" />
          <span className="min-w-0 truncate">{triggerLabel}</span>
          {activeModified && (
            <Circle
              className="size-1.5! fill-warn text-warn"
              aria-label="view modified"
            />
          )}
          <ChevronDown
            data-icon="inline-end"
            className="size-4 text-muted-foreground"
          />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-56 rounded-lg">
          <DropdownMenuGroup>
            <DropdownMenuLabel>Views</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={activeViewId ?? ''}
              onValueChange={(id) =>
                onSelectView(views.find((v) => v.id === id) ?? null)
              }
            >
              <DropdownMenuRadioItem value="">All traces</DropdownMenuRadioItem>
              {views.map((view) => (
                <DropdownMenuRadioItem key={view.id} value={view.id}>
                  <span className="truncate">{view.name}</span>
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <DropdownMenuItem onClick={() => openDialog('save')}>
              Save current as view…
            </DropdownMenuItem>
            {activeView && (
              <>
                <DropdownMenuItem
                  disabled={!activeModified}
                  onClick={onUpdateActive}
                >
                  Update “{activeView.name}”
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => openDialog('rename')}>
                  Rename…
                </DropdownMenuItem>
                <DropdownMenuItem
                  variant="destructive"
                  onClick={onDeleteActive}
                >
                  Delete view
                </DropdownMenuItem>
              </>
            )}
          </DropdownMenuGroup>
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
