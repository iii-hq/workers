import { Blocks, Check, ChevronDown } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { useMediaQuery } from '@/hooks/use-media-query'
import { listSkills } from '@/lib/backend/directory-prompts'
import type { IiiClient } from '@/lib/iii-client'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import {
  type SkillSelection,
  toggleSkillSelection,
} from './system-prompt-selection'

/**
 * Multi-select for the welcome screen's model-invocable skill IDs. A sibling
 * of `SystemPromptPicker`, not a generalization —
 * multi-select needs checkbox rows, which Radix Select can't do.
 */
export async function loadSessionSkills(client: IiiClient) {
  return (await listSkills(client)).filter(
    (skill) => !skill.disable_model_invocation,
  )
}

interface SessionAddonsPickerPanelProps {
  value: SkillSelection
  entries: { name: string; description: string }[] | null
  disabled?: boolean
  onClear: () => void
  onToggle: (name: string) => void
}

/** Multi-select skill list used by the mobile sheet. */
export function SessionAddonsPickerPanel({
  value,
  entries,
  disabled,
  onClear,
  onToggle,
}: SessionAddonsPickerPanelProps) {
  const selected = new Set(value ?? [])
  return (
    <div className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto px-3 pb-1">
      <fieldset
        disabled={disabled}
        className="w-full min-w-0 max-w-full divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge"
      >
        <legend className="sr-only">Session skills</legend>
        <button
          type="button"
          aria-pressed={selected.size === 0}
          disabled={disabled}
          onClick={onClear}
          className={cn(
            'flex min-h-14 w-full min-w-0 items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40',
            selected.size === 0 && 'bg-surface-selected',
          )}
        >
          <span className="min-w-0 flex-1 font-medium">All skills</span>
          {selected.size === 0 ? (
            <Check className="size-5 shrink-0 text-ink" aria-hidden />
          ) : null}
        </button>

        {entries === null ? (
          <div className="px-3 py-3 font-sans text-sm text-ink-faint">
            Loading skills…
          </div>
        ) : entries.length === 0 ? (
          <div className="px-3 py-3 font-sans text-sm text-ink-faint">
            No skills available
          </div>
        ) : (
          entries.map((entry) => {
            const checked = selected.has(entry.name)
            return (
              <button
                key={entry.name}
                type="button"
                aria-pressed={checked}
                disabled={disabled}
                onClick={() => onToggle(entry.name)}
                className={cn(
                  'flex min-h-14 w-full min-w-0 items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40',
                  checked && 'bg-surface-selected',
                )}
              >
                <span className="flex min-w-0 flex-1 flex-col overflow-hidden">
                  <span className="truncate font-medium">{entry.name}</span>
                  {entry.description ? (
                    <span className="truncate text-sm leading-relaxed text-ink-faint">
                      {entry.description}
                    </span>
                  ) : null}
                </span>
                {checked ? (
                  <Check className="size-5 shrink-0 text-ink" aria-hidden />
                ) : null}
              </button>
            )
          })
        )}
      </fieldset>
    </div>
  )
}

export function SessionAddonsPicker({
  value,
  onChange,
  disabled,
  appearance = 'default',
  className,
}: {
  value: SkillSelection
  onChange: (next: SkillSelection) => void
  disabled?: boolean
  /** Text-only action used by the compact empty-state session controls. */
  appearance?: 'default' | 'inline'
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const [entries, setEntries] = useState<
    { name: string; description: string }[] | null
  >(null)
  const mobileSheet = useMediaQuery('(max-width: 767px)')

  /* Radix keeps the menu open across toggles, so update the ref immediately
     rather than waiting for the parent render between adjacent clicks. */
  const valueRef = useRef(value)
  valueRef.current = value

  /* Fetch on EVERY open, SystemPromptPicker-style: keep the last list while
     loading, degrade to an empty list when the directory worker is absent. */
  const handleOpenChange = useCallback(async (nextOpen: boolean) => {
    setOpen(nextOpen)
    if (!nextOpen) return
    try {
      const client = await getIiiClient()
      setEntries(
        (await loadSessionSkills(client)).map((s) => ({
          name: s.id,
          description: s.description || s.title,
        })),
      )
    } catch {
      setEntries((prev) => prev ?? [])
    }
  }, [])

  useEffect(() => {
    if (disabled && open) setOpen(false)
  }, [disabled, open])

  const isChecked = useCallback(
    (name: string) => value?.includes(name) ?? false,
    [value],
  )

  const toggle = useCallback(
    (name: string) => {
      const next = toggleSkillSelection(valueRef.current, name)
      valueRef.current = next
      onChange(next)
    },
    [onChange],
  )

  const count = value?.length ?? 0

  const triggerClassName = cn(
    appearance === 'inline'
      ? 'relative inline-flex min-h-5.5 min-w-0 items-center border-dashed border-b border-ink-faint/50 px-0.5 font-sans text-base font-medium text-ink hover:border-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:border-ink sm:text-[0.8125rem]'
      : 'inline-flex h-9 w-full items-center justify-between gap-x-2 rounded-sm border border-transparent bg-bg px-3 font-sans text-[0.8125rem] text-ink transition-colors hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-active',
    disabled && 'pointer-events-none opacity-40',
    className,
  )

  const triggerContent =
    appearance === 'inline' ? (
      <>
        <span>Add skills</span>
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
      </>
    ) : (
      <>
        <span className="inline-flex min-w-0 items-center gap-2">
          <Blocks size={16} className="shrink-0 text-ink-faint" aria-hidden />
          <span className="truncate">
            {count > 0 ? `Skills (${count})` : 'All skills'}
          </span>
        </span>
        <ChevronDown className="size-4 shrink-0" aria-hidden />
      </>
    )

  if (mobileSheet) {
    return (
      <>
        <button
          type="button"
          aria-label={
            count > 0
              ? `add skills to this session, ${count} selected`
              : 'add skills to this session'
          }
          aria-haspopup="dialog"
          aria-expanded={open}
          data-state={open ? 'open' : 'closed'}
          disabled={disabled}
          onClick={() => void handleOpenChange(!open)}
          className={triggerClassName}
        >
          {triggerContent}
        </button>
        <BottomSheet open={open} onOpenChange={handleOpenChange}>
          <BottomSheetContent heading="Skills" closeLabel="Close skills picker">
            <SessionAddonsPickerPanel
              value={value}
              entries={entries}
              disabled={disabled}
              onClear={() => {
                valueRef.current = undefined
                onChange(undefined)
              }}
              onToggle={toggle}
            />
          </BottomSheetContent>
        </BottomSheet>
      </>
    )
  }

  return (
    <DropdownMenu open={open} onOpenChange={handleOpenChange}>
      <DropdownMenuTrigger
        aria-label={
          count > 0
            ? `add skills to this session, ${count} selected`
            : 'add skills to this session'
        }
        disabled={disabled}
        className={triggerClassName}
      >
        {triggerContent}
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        collisionPadding={8}
        /* Width is anchored to the trigger and CAPPED: descriptions are full
           sentences, and an uncapped popper sizes to the widest one — a
           viewport-wide panel that clips its own checkmarks at the screen
           edge. The cap is what makes the row-level truncation engage. */
        className="min-w-[var(--radix-dropdown-menu-trigger-width)] max-w-[min(24rem,var(--radix-dropdown-menu-content-available-width))] max-h-[min(18rem,var(--radix-dropdown-menu-content-available-height))] overflow-x-hidden overflow-y-auto text-[13px]"
      >
        <DropdownMenuCheckboxItem
          className="min-w-0"
          checked={count === 0}
          onSelect={(ev) => ev.preventDefault()}
          onCheckedChange={() => {
            valueRef.current = undefined
            onChange(undefined)
          }}
          indicator={
            <Check
              size={16}
              strokeWidth={2.5}
              className="text-ink"
              aria-hidden
            />
          }
        >
          All skills
        </DropdownMenuCheckboxItem>
        {entries === null ? (
          <DropdownMenuItem disabled>Loading skills…</DropdownMenuItem>
        ) : entries.length === 0 ? (
          <DropdownMenuItem disabled>No skills available</DropdownMenuItem>
        ) : (
          entries.map((e) => (
            <DropdownMenuCheckboxItem
              key={e.name}
              className="min-w-0"
              checked={isChecked(e.name)}
              /* Keep the menu open across toggles: multi-select. */
              onSelect={(ev) => ev.preventDefault()}
              onCheckedChange={() => toggle(e.name)}
              /* Selection indicators stay neutral across themes. */
              indicator={
                <Check
                  size={16}
                  strokeWidth={2.5}
                  className="text-ink"
                  aria-hidden
                />
              }
            >
              <div
                className="min-w-0 flex-1 overflow-hidden"
                title={e.description}
              >
                <div className="truncate">{e.name}</div>
                {e.description ? (
                  <div className="truncate text-[11px] leading-[1.5] text-ink-faint">
                    {e.description}
                  </div>
                ) : null}
              </div>
            </DropdownMenuCheckboxItem>
          ))
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
