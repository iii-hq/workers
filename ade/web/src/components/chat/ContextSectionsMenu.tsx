import uiClasses from '@iii-dev/console-ui/ui-classes'
import { SlidersHorizontal } from 'lucide-react'
import { useState } from 'react'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { Checkbox } from '@/components/ui/Checkbox'
import { IconButton } from '@/components/ui/IconButton'
import { LiveRegion } from '@/components/ui/LiveRegion'
import { useLiveAnnouncer } from '@/hooks/use-live-announcer'
import {
  allGroupsFolded,
  foldAllGroups,
  moveSection,
  resetSectionOrder,
  setSectionHidden,
  useContextSections,
  useFoldedGroups,
  useHiddenSections,
} from '@/lib/context-groups'
import { cn } from '@/lib/utils'
import { ReorderHandle, type ReorderTarget } from './ReorderHandle'

/**
 * Which Context sections show, and in what order: every section the panel
 * can show, the panel's own and the workers', ticked when shown, with a
 * handle to move it. A popover on a desktop, a sheet on a phone (where the
 * sheet also folds every section, and dragging in the panel would scroll).
 */
export function ContextSectionsMenu({ phone }: { phone: boolean }) {
  const [open, setOpen] = useState(false)
  const sections = useContextSections()
  const hidden = useHiddenSections()
  const ids = sections.map((section) => section.id)
  const allFolded = allGroupsFolded(useFoldedGroups(), ids)
  const { announcement, announce } = useLiveAnnouncer()
  const name = (id: string) =>
    sections.find((section) => section.id === id)?.label ?? id
  const onMove = (id: string, target: ReorderTarget) => {
    moveSection(ids, id, target)
    announce(
      'before' in target
        ? `${name(id)}, now above ${name(target.before)}`
        : `${name(id)}, now below ${name(target.after)}`,
    )
  }

  const list = (
    <ul
      data-reorder-list=""
      aria-label="Sections, in panel order"
      className="flex flex-col"
    >
      {sections.map((section) => {
        const handle = (
          <ReorderHandle
            label={section.label}
            onMove={(target) => onMove(section.id, target)}
            className={phone ? 'size-11' : 'h-7 w-6'}
          />
        )
        return (
          <li
            key={section.id}
            data-reorder-item={section.id}
            className={cn(
              'flex items-center gap-1 rounded-sm data-[dragging]:relative data-[dragging]:z-10 data-[dragging]:bg-panel-raised data-[dragging]:shadow-floating data-[dragging]:ring-1 data-[dragging]:ring-ring/50',
              phone ? 'h-12 pl-3' : 'h-8 pr-2',
            )}
          >
            {phone ? null : handle}
            <Checkbox
              checked={!hidden.has(section.id)}
              onChange={(event) =>
                setSectionHidden(section.id, !event.target.checked)
              }
              className="min-w-0 flex-1"
              label={
                <span className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 flex-1 truncate">
                    {section.label}
                  </span>
                  {section.source ? (
                    <span className="shrink-0 font-mono text-xs text-ink-faint">
                      {section.source}
                    </span>
                  ) : null}
                </span>
              }
            />
            {phone ? handle : null}
          </li>
        )
      })}
    </ul>
  )
  const reset = (
    <button
      type="button"
      onClick={resetSectionOrder}
      className={cn(
        'flex items-center rounded-sm px-2 text-left text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50',
        phone ? 'h-12 px-3' : 'h-8',
      )}
    >
      Reset to default order
    </button>
  )

  if (phone) {
    return (
      <>
        <IconButton
          label="Sections"
          className="size-11 shrink-0"
          onClick={() => setOpen(true)}
        >
          <SlidersHorizontal aria-hidden />
        </IconButton>
        <BottomSheet open={open} onOpenChange={setOpen}>
          <BottomSheetContent
            heading="Sections"
            description="Hold a handle to move a section."
            className="mx-auto max-w-[460px] font-inter"
          >
            <div className="flex flex-col gap-1 px-2 pb-1">
              <button
                type="button"
                onClick={() => foldAllGroups(!allFolded, ids)}
                className="flex h-11 items-center self-start rounded-sm bg-surface-hover px-3 text-sm hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
              >
                {allFolded ? 'Expand all' : 'Collapse all'}
              </button>
              {list}
              {reset}
            </div>
            <LiveRegion announcement={announcement} />
          </BottomSheetContent>
        </BottomSheet>
      </>
    )
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <IconButton label="Sections" className="size-7 shrink-0">
            <SlidersHorizontal aria-hidden />
          </IconButton>
        }
      />
      <PopoverContent
        align="end"
        sideOffset={4}
        collisionPadding={8}
        aria-label="Sections"
        className={cn(
          uiClasses.panel,
          'w-72 max-w-[calc(100vw-1rem)] gap-0 p-1.5',
        )}
      >
        <div className="flex items-baseline gap-2 px-2 pt-1 pb-1.5">
          <span className="font-semibold">Sections</span>
          <span className="text-xs text-ink-faint">Drag to reorder</span>
        </div>
        {list}
        <div className="-mx-1.5 my-1 h-px bg-border" />
        {reset}
        <LiveRegion announcement={announcement} />
      </PopoverContent>
    </Popover>
  )
}
