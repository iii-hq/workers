import { Blocks, Check, ChevronDown } from 'lucide-react'
import { useCallback, useRef, useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { getSkill, listSkills } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import type { SystemPromptState } from './system-prompt-selection'

/**
 * Multi-select for the welcome screen's session skills: directory skills
 * whose bodies are appended to the session's system prompt on the first
 * send. A sibling of `SystemPromptPicker`, not a generalization —
 * multi-select needs checkbox rows, which Radix Select can't do.
 *
 * Skills only: command prompts are user-invoked `/` commands in the
 * composer, not session-start context. The `prompt` addon kind survives in
 * the state model solely so metadata persisted by older builds still
 * decodes and sends.
 */
export function SessionAddonsPicker({
  value,
  onChange,
  disabled,
  className,
}: {
  value: SystemPromptState
  onChange: (next: SystemPromptState) => void
  disabled?: boolean
  className?: string
}) {
  const [entries, setEntries] = useState<
    { name: string; description: string }[] | null
  >(null)

  /* The menu stays open across toggles and body resolution awaits the bus,
     so the post-await state must come from the LATEST value — a closure
     captured before the await would drop or duplicate concurrent toggles. */
  const valueRef = useRef(value)
  valueRef.current = value

  /* Fetch on EVERY open, SystemPromptPicker-style: keep the last list while
     loading, degrade to an empty list when the directory worker is absent. */
  const handleOpenChange = useCallback(async (open: boolean) => {
    if (!open) return
    try {
      const client = await getIiiClient()
      setEntries(
        (await listSkills(client)).map((s) => ({
          name: s.id,
          description: s.description || s.title,
        })),
      )
    } catch {
      setEntries((prev) => prev ?? [])
    }
  }, [])

  const isChecked = useCallback(
    (name: string) =>
      value.addons.some((a) => a.kind === 'skill' && a.name === name),
    [value.addons],
  )

  const toggle = useCallback(
    async (name: string) => {
      const cur = valueRef.current
      if (cur.addons.some((a) => a.kind === 'skill' && a.name === name)) {
        onChange({
          ...cur,
          addons: cur.addons.filter(
            (a) => !(a.kind === 'skill' && a.name === name),
          ),
        })
        return
      }
      /* Resolve the body at selection time — frozen server-side on the
         first send, same contract as the identity prompt's namedBody. */
      try {
        const client = await getIiiClient()
        const body = (await getSkill(client, name)).body
        const latest = valueRef.current
        if (latest.addons.some((a) => a.kind === 'skill' && a.name === name)) {
          return
        }
        onChange({
          ...latest,
          addons: [...latest.addons, { kind: 'skill', name, body }],
        })
      } catch {
        /* Unresolvable body = nothing to add; leave the selection as-is. */
      }
    },
    [onChange],
  )

  const count = value.addons.filter((a) => a.kind === 'skill').length

  return (
    <DropdownMenu onOpenChange={handleOpenChange}>
      <DropdownMenuTrigger
        aria-label="session skills"
        disabled={disabled}
        className={cn(
          'inline-flex w-full items-center justify-between gap-x-2 rounded-sm border border-transparent bg-bg px-3 h-9 text-ink font-mono text-[13px] lowercase hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-active transition-colors',
          disabled && 'opacity-40 pointer-events-none',
          className,
        )}
      >
        <span className="inline-flex items-center gap-2 min-w-0">
          <Blocks size={14} className="text-ink-faint" aria-hidden />
          <span className="truncate">
            {count > 0 ? `skills (${count})` : 'skills'}
          </span>
        </span>
        <ChevronDown size={12} aria-hidden />
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
        {entries === null ? (
          <DropdownMenuItem disabled>loading skills…</DropdownMenuItem>
        ) : entries.length === 0 ? (
          <DropdownMenuItem disabled>no skills available</DropdownMenuItem>
        ) : (
          entries.map((e) => (
            <DropdownMenuCheckboxItem
              key={e.name}
              checked={isChecked(e.name)}
              /* Keep the menu open across toggles: multi-select. */
              onSelect={(ev) => ev.preventDefault()}
              onCheckedChange={() => void toggle(e.name)}
              /* Same accent ✓ as the StrategyToggle beside this picker. */
              indicator={
                <Check
                  size={12}
                  strokeWidth={2.5}
                  className="text-accent"
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
