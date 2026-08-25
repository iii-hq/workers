import { Blocks, Check, ChevronDown } from 'lucide-react'
import { useCallback, useRef, useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
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

export function SessionAddonsPicker({
  value,
  onChange,
  disabled,
  className,
}: {
  value: SkillSelection
  onChange: (next: SkillSelection) => void
  disabled?: boolean
  className?: string
}) {
  const [entries, setEntries] = useState<
    { name: string; description: string }[] | null
  >(null)

  /* Radix keeps the menu open across toggles, so update the ref immediately
     rather than waiting for the parent render between adjacent clicks. */
  const valueRef = useRef(value)
  valueRef.current = value

  /* Fetch on EVERY open, SystemPromptPicker-style: keep the last list while
     loading, degrade to an empty list when the directory worker is absent. */
  const handleOpenChange = useCallback(async (open: boolean) => {
    if (!open) return
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

  return (
    <DropdownMenu onOpenChange={handleOpenChange}>
      <DropdownMenuTrigger
        aria-label="session skills"
        disabled={disabled}
        className={cn(
          'inline-flex w-full items-center justify-between gap-x-2 rounded-sm border border-transparent bg-bg px-3 h-9 text-ink font-sans text-[13px] hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-active transition-colors',
          disabled && 'opacity-40 pointer-events-none',
          className,
        )}
      >
        <span className="inline-flex items-center gap-2 min-w-0">
          <Blocks size={16} className="text-ink-faint" aria-hidden />
          <span className="truncate">
            {count > 0 ? `Skills (${count})` : 'All skills'}
          </span>
        </span>
        <ChevronDown size={16} aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        collisionPadding={8}
        /* Width is anchored to the trigger and CAPPED: descriptions are full
           sentences, and an uncapped popper sizes to the widest one — a
           viewport-wide panel that clips its own checkmarks at the screen
           edge. The cap is what makes the row-level truncation engage. */
        className="min-w-[var(--radix-dropdown-menu-trigger-width)] max-w-[min(24rem,var(--radix-dropdown-menu-content-available-width))] max-h-[min(18rem,var(--radix-dropdown-menu-content-available-height))] overflow-y-auto text-[13px]"
      >
        <DropdownMenuCheckboxItem
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
              <div className="min-w-0 flex-1" title={e.description}>
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
