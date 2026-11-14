import { Select as SelectPrimitive } from '@base-ui/react/select'
import { Bot, Check } from 'lucide-react'
import { useCallback, useState } from 'react'
import {
  SelectContent,
  SelectTrigger,
  SelectValue,
} from '@/components/shadcn/select'
import { type AgentEntry, listAgents } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import type { SubagentIcon } from '@/types/chat'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import {
  AGENT_CHOICE_PREFIX,
  agentIdFromSystemPrompt,
  type SystemPromptState,
  withAgentChoice,
} from './system-prompt-selection'

/**
 * The new-session agent picker: "Default" (the provider's built-in prompt)
 * or one of the reusable agent profiles the iii-directory worker serves.
 *
 * The picker stores only the id (`choice: { named: 'agent:<id>' }`); the
 * first send carries it as `harness::send` `options.agent` and the harness
 * resolves the profile server-side (MOT-4485) — prompt, preloaded skills and
 * functions, and default model all come from the frozen profile.
 */

export { AGENT_CHOICE_PREFIX }

function agentIcon(icon: string | null): React.ComponentType<{
  size?: number
  className?: string
  'aria-hidden'?: boolean
}> {
  return (icon && SUBAGENT_ICON_COMPONENTS[icon as SubagentIcon]) || Bot
}

function AgentItem({
  value,
  label,
  description,
  icon,
}: {
  value: string
  label: string
  description?: string
  icon?: string | null
}) {
  const Icon = agentIcon(icon ?? null)
  return (
    <SelectPrimitive.Item
      value={value}
      className={cn(
        'relative flex cursor-pointer items-start gap-2 rounded-md py-2 pr-8 pl-2 outline-none select-none',
        'data-highlighted:bg-muted data-highlighted:text-foreground',
        'data-selected:text-foreground',
      )}
    >
      <Icon size={16} className="mt-0.5 shrink-0 text-ink-faint" aria-hidden />
      <div className="min-w-0">
        <SelectPrimitive.ItemText>{label}</SelectPrimitive.ItemText>
        {description ? (
          <div className="truncate text-xs leading-[1.5] text-ink-faint">
            {description}
          </div>
        ) : null}
      </div>
      <SelectPrimitive.ItemIndicator
        render={
          <span className="pointer-events-none absolute top-2.5 right-2 flex size-4 items-center justify-center text-ink" />
        }
      >
        <Check size={16} aria-hidden />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  )
}

export function AgentPicker({
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
  const [entries, setEntries] = useState<AgentEntry[] | null>(null)

  /* Fetch on EVERY open (keeping the last list while loading): an agent
     authored in the directory UI shows up on the next open with no
     cache-invalidation plumbing. A failed load degrades to Default only
     (directory worker absent ≠ broken chat). */
  const handleOpenChange = useCallback(async (open: boolean) => {
    if (!open) return
    try {
      setEntries(await listAgents(await getIiiClient()))
    } catch {
      setEntries((prev) => prev ?? [])
    }
  }, [])

  const selectedId = agentIdFromSystemPrompt(value)

  const handleValueChange = useCallback(
    (v: string) => {
      if (v === 'default') {
        onChange({ ...value, choice: 'default', namedBody: '' })
        return
      }
      /* Store only the id; the harness resolves the profile on the first
         send (`options.agent`) and freezes it server-side. */
      const id = v.slice(`named:${AGENT_CHOICE_PREFIX}`.length)
      onChange(withAgentChoice(value, id))
    },
    [onChange, value],
  )

  const selectedEntry = entries?.find((e) => e.id === selectedId)
  const label =
    selectedId === null ? 'Default' : (selectedEntry?.name ?? selectedId)
  const TriggerIconComp = agentIcon(selectedEntry?.icon ?? null)

  return (
    <SelectPrimitive.Root
      value={
        selectedId === null
          ? 'default'
          : `named:${AGENT_CHOICE_PREFIX}${selectedId}`
      }
      onValueChange={(next) => handleValueChange(next ?? 'default')}
      onOpenChange={handleOpenChange}
      disabled={disabled}
    >
      <SelectTrigger
        aria-label="agent profile"
        className={cn(
          'h-9 w-full justify-between border-transparent bg-bg px-3 font-inter hover:bg-surface-hover data-popup-open:bg-surface-active',
          disabled && 'pointer-events-none opacity-40',
          className,
        )}
      >
        <span className="inline-flex min-w-0 items-center gap-2">
          <TriggerIconComp size={16} className="text-ink-faint" aria-hidden />
          <span className="max-w-[14rem] truncate">
            <SelectValue>{label}</SelectValue>
          </span>
        </span>
      </SelectTrigger>

      <SelectContent align="start" alignItemWithTrigger={false}>
        <AgentItem
          value="default"
          label="Default"
          description="No agent profile — the provider's built-in prompt"
        />
        {(entries ?? []).map((e) => (
          <AgentItem
            key={e.id}
            value={`named:${AGENT_CHOICE_PREFIX}${e.id}`}
            label={e.name || e.id}
            description={e.description}
            icon={e.icon}
          />
        ))}
      </SelectContent>
    </SelectPrimitive.Root>
  )
}
