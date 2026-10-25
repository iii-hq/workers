/**
 * The trace views' span filter: a funnel button that opens a menu (on
 * hover as well as click/keyboard — hover is sugar, not the only path).
 * Three sections: "Workers" hides a worker's own spans, "Spans" hides one
 * span group, and "Internal" toggles the call-site-tagged plumbing families
 * (`iii.tag.hidden` — hidden by DEFAULT, so entries start checked). All
 * list entries most-populated first. Hiding removes ONLY the matched spans
 * — their children stay visible, re-attached to the hidden span's parent
 * (see `spanVisibility.ts`). Built on the shadcn dropdown-menu (Base UI),
 * whose trigger opens on hover natively and whose checkbox items can stay
 * open across toggles.
 */

import { EyeOff, Funnel } from 'lucide-react'
import { Button } from '@/components/shadcn/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { cn } from '@/lib/utils'
import type { SpanGroup } from './spanVisibility'

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
      {groups.map((group) => {
        const hidden = hiddenKeys.has(group.key)
        return (
          <DropdownMenuCheckboxItem
            key={group.key}
            checked={hidden}
            onCheckedChange={() => onToggle(group.key)}
            // Keep the menu open across toggles — filtering is multi-select.
            closeOnClick={false}
            // Checked here means HIDDEN — the eye-off reads that state
            // better than a checkmark would.
            indicator={<EyeOff aria-hidden className="size-4" />}
            title={
              hidden
                ? `hidden — click to show ${group.key}`
                : `hide ${group.key} spans (their children stay, re-attached to the parent)`
            }
            className={cn(hidden && 'text-muted-foreground')}
          >
            <span className="min-w-0 flex-1 truncate font-mono">
              {group.key}
            </span>
            <span className="ml-3 shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
              {group.count}
            </span>
          </DropdownMenuCheckboxItem>
        )
      })}
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
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger
        openOnHover
        delay={120}
        render={
          <Button
            type="button"
            variant={filtering ? 'secondary' : 'ghost'}
            size="xs"
            aria-label={
              filtering
                ? `filter spans — ${hiddenSpanCount} hidden`
                : 'filter spans'
            }
            title={
              filtering
                ? `${hiddenSpanCount} spans hidden by the filter`
                : 'Hide workers or span groups'
            }
            className={cn('gap-1 px-1.5 font-mono tabular-nums', className)}
          />
        }
      >
        <Funnel className="size-4" aria-hidden />
        {filtering ? <span>{hiddenSpanCount}</span> : null}
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="max-h-72 w-64 overflow-y-auto overscroll-contain rounded-lg"
      >
        <FilterSection
          label="Workers"
          groups={workerGroups}
          hiddenKeys={hiddenWorkerKeys}
          onToggle={onToggleWorker}
        />
        {workerGroups.length > 0 && groups.length > 0 && (
          <DropdownMenuSeparator />
        )}
        <FilterSection
          label="Spans"
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
            label="Internal"
            groups={internalGroups}
            hiddenKeys={hiddenInternalKeys}
            onToggle={onToggleInternal}
          />
        )}
        {anyHidden && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem closeOnClick={false} onClick={onClear}>
              Show all spans
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
