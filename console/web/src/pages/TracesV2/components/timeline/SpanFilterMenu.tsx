/**
 * The trace detail views' filter: a funnel button that expands on hover
 * into a dropdown menu (also opens on click/keyboard — hover is sugar, not
 * the only path). Three sections: "workers" hides a worker's own spans,
 * "spans" hides one span group, and "internal" toggles the call-site-tagged
 * plumbing families (`iii.tag.hidden` — hidden by DEFAULT, so entries start
 * checked). All list entries most-populated first. Hiding removes ONLY the
 * matched spans — their children stay visible, re-attached to the hidden
 * span's parent (see `spanVisibility.ts`). Built on the design-system
 * `DropdownMenu` (the shadcn anatomy over Radix).
 */

import { EyeOff, Funnel } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { cn } from '@/lib/utils'
import type { SpanGroup } from './spanVisibility'

/** Grace period so the pointer can travel from the trigger to the popup. */
const HOVER_CLOSE_DELAY_MS = 150

interface FilterSectionProps {
  label: string
  /** Menu entries, already ranked (see `deriveSpanGroups`). */
  groups: readonly SpanGroup[]
  /** Group keys currently hidden — rendered as checked items. */
  hiddenKeys: ReadonlySet<string>
  onToggle: (key: string) => void
}

function FilterSection({
  label,
  groups,
  hiddenKeys,
  onToggle,
}: FilterSectionProps) {
  if (groups.length === 0) return null
  return (
    <DropdownMenuGroup>
      <DropdownMenuLabel>{label}</DropdownMenuLabel>
      {groups.map((group) => (
        <DropdownMenuCheckboxItem
          key={group.key}
          checked={hiddenKeys.has(group.key)}
          onCheckedChange={() => onToggle(group.key)}
          // Keep the menu open across toggles — filtering is multi-select.
          onSelect={(e) => e.preventDefault()}
          // Checked here means HIDDEN — the eye-off reads that state
          // better than a checkmark would.
          indicator={<EyeOff aria-hidden className="h-3 w-3" />}
          title={
            hiddenKeys.has(group.key)
              ? `hidden — click to show ${group.key}`
              : `hide ${group.key} spans (their children stay, re-attached to the parent)`
          }
        >
          <span className="min-w-0 flex-1 truncate">{group.key}</span>
          <span className="ml-2 shrink-0 text-ink-faint tabular-nums">
            {group.count}
          </span>
        </DropdownMenuCheckboxItem>
      ))}
    </DropdownMenuGroup>
  )
}

export interface SpanFilterMenuProps {
  /** Span-group entries, already ranked (see `deriveSpanGroups`). */
  groups: readonly SpanGroup[]
  /** Worker entries, already ranked (`deriveSpanGroups` + `workerGroupKey`). */
  workerGroups: readonly SpanGroup[]
  /** Internal-family entries (`iii.tag.hidden` values), already ranked.
   *  Hidden by DEFAULT — an entry renders unchecked only when its family
   *  is in `shownInternalKeys`. */
  internalGroups?: readonly SpanGroup[]
  /** Span-group keys currently hidden. */
  hiddenKeys: ReadonlySet<string>
  /** Worker names currently hidden. */
  hiddenWorkerKeys: ReadonlySet<string>
  /** Internal families the user revealed. */
  shownInternalKeys?: ReadonlySet<string>
  /** Spans removed from the view right now (badge on the funnel). */
  hiddenSpanCount: number
  onToggle: (key: string) => void
  onToggleWorker: (key: string) => void
  onToggleInternal?: (family: string) => void
  onClear: () => void
  className?: string
}

export function SpanFilterMenu({
  groups,
  workerGroups,
  internalGroups = [],
  hiddenKeys,
  hiddenWorkerKeys,
  shownInternalKeys,
  hiddenSpanCount,
  onToggle,
  onToggleWorker,
  onToggleInternal,
  onClear,
  className,
}: SpanFilterMenuProps) {
  const [open, setOpen] = useState(false)
  const closeTimer = useRef<number | null>(null)

  const cancelClose = () => {
    if (closeTimer.current != null) {
      window.clearTimeout(closeTimer.current)
      closeTimer.current = null
    }
  }
  const openNow = () => {
    cancelClose()
    setOpen(true)
  }
  const scheduleClose = () => {
    cancelClose()
    closeTimer.current = window.setTimeout(
      () => setOpen(false),
      HOVER_CLOSE_DELAY_MS,
    )
  }
  // Clear any pending hover-close when unmounting.
  useEffect(
    () => () => {
      if (closeTimer.current != null) window.clearTimeout(closeTimer.current)
    },
    [],
  )

  // Internal families are hidden unless explicitly shown — derive the
  // checked (hidden) set for the section.
  const hiddenInternalKeys = new Set(
    internalGroups
      .map((g) => g.key)
      .filter((key) => !(shownInternalKeys?.has(key) ?? false)),
  )

  const filtering = hiddenSpanCount > 0
  const anyHidden =
    hiddenKeys.size > 0 ||
    hiddenWorkerKeys.size > 0 ||
    hiddenInternalKeys.size > 0
  if (
    groups.length === 0 &&
    workerGroups.length === 0 &&
    internalGroups.length === 0 &&
    !filtering
  )
    return null

  return (
    <DropdownMenu open={open} onOpenChange={setOpen} modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="filter spans"
          onPointerEnter={openNow}
          onPointerLeave={scheduleClose}
          className={cn(
            'flex items-center gap-1 border px-1.5 py-1 font-mono text-[10px] lowercase backdrop-blur-sm transition-colors',
            filtering
              ? 'border-accent bg-panel text-accent'
              : 'border-rule bg-bg/90 text-ink-faint hover:text-ink',
            className,
          )}
        >
          <Funnel className="h-3 w-3" />
          {filtering && <span className="tabular-nums">{hiddenSpanCount}</span>}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onPointerEnter={cancelClose}
        onPointerLeave={scheduleClose}
        className="max-h-64 min-w-[14rem] overflow-y-auto"
      >
        <FilterSection
          label="workers"
          groups={workerGroups}
          hiddenKeys={hiddenWorkerKeys}
          onToggle={onToggleWorker}
        />
        {workerGroups.length > 0 && groups.length > 0 && (
          <DropdownMenuSeparator />
        )}
        <FilterSection
          label="spans"
          groups={groups}
          hiddenKeys={hiddenKeys}
          onToggle={onToggle}
        />
        {internalGroups.length > 0 &&
          onToggleInternal &&
          (workerGroups.length > 0 || groups.length > 0) && (
            <DropdownMenuSeparator />
          )}
        {onToggleInternal && (
          <FilterSection
            label="internal"
            groups={internalGroups}
            hiddenKeys={hiddenInternalKeys}
            onToggle={onToggleInternal}
          />
        )}
        {anyHidden && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={(e) => {
                e.preventDefault()
                onClear()
              }}
            >
              show all
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
