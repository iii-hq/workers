import * as SelectPrimitive from '@radix-ui/react-select'
import { Bot, Check, ChevronDown } from 'lucide-react'
import { useCallback, useState } from 'react'
import {
  type AgentEntry,
  getAgent,
  listAgents,
} from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import type { SkillSelection, SystemPromptState } from './system-prompt-selection'
import type { SubagentIcon } from '@/types/chat'

/**
 * The new-session agent picker: "Default" (the provider's built-in prompt)
 * or one of the reusable agent profiles the iii-directory worker serves.
 *
 * Picking an agent maps onto the machinery the system-prompt picker
 * already established (`Conversation.systemPrompt`, frozen on the first
 * send): the profile's body becomes the named prompt under the `enrich`
 * strategy with a `You are <name>.` line, and the profile's skill filter
 * becomes the session's skill selection. Until the harness resolves
 * `options.agent` natively (MOT-4485), this is the resolution.
 */

/** `agent:` inside the named choice keeps agent selections distinct from
 * fs system prompts (prompt names are `[a-z0-9_-]`, so the prefix is
 * unambiguous). */
export const AGENT_CHOICE_PREFIX = 'agent:'

function agentIcon(icon: string | null): React.ComponentType<{
  size?: number
  className?: string
  'aria-hidden'?: boolean
}> {
  return (
    (icon && SUBAGENT_ICON_COMPONENTS[icon as SubagentIcon]) ||
    Bot
  )
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
        'relative flex items-start gap-2 rounded-xs pl-7 pr-3 py-2 cursor-pointer outline-none select-none',
        'data-[highlighted]:bg-surface-hover data-[highlighted]:text-ink',
        'data-[state=checked]:text-ink',
      )}
    >
      <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
        <Check size={16} aria-hidden />
      </SelectPrimitive.ItemIndicator>
      <Icon size={16} className="mt-0.5 shrink-0 text-ink-faint" aria-hidden />
      <div className="min-w-0">
        <SelectPrimitive.ItemText>{label}</SelectPrimitive.ItemText>
        {description ? (
          <div className="truncate text-[11px] leading-[1.5] text-ink-faint">
            {description}
          </div>
        ) : null}
      </div>
    </SelectPrimitive.Item>
  )
}

export function AgentPicker({
  value,
  onChange,
  onSkillsChange,
  disabled,
  className,
}: {
  value: SystemPromptState
  onChange: (next: SystemPromptState) => void
  onSkillsChange?: (next: SkillSelection) => void
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
      /* Leaf agents are spawn targets for orchestrators, not session
         identities — the picker offers only agents that may delegate. */
      setEntries(
        (await listAgents(await getIiiClient())).filter((e) => !e.leaf),
      )
    } catch {
      setEntries((prev) => prev ?? [])
    }
  }, [])

  const selectedId =
    typeof value.choice === 'object' &&
    value.choice.named.startsWith(AGENT_CHOICE_PREFIX)
      ? value.choice.named.slice(AGENT_CHOICE_PREFIX.length)
      : null

  const handleValueChange = useCallback(
    async (v: string) => {
      if (v === 'default') {
        onChange({ ...value, choice: 'default', namedBody: '' })
        onSkillsChange?.(undefined)
        return
      }
      const id = v.slice(`named:${AGENT_CHOICE_PREFIX}`.length)
      try {
        /* Resolve the profile at selection time; the first send freezes it
           server-side, same contract as the named system prompts. */
        const agent = await getAgent(await getIiiClient(), id)
        onChange({
          ...value,
          choice: { named: `${AGENT_CHOICE_PREFIX}${id}` },
          namedBody: `You are ${agent.name || id}.\n\n${agent.system_prompt}`,
          strategy: 'enrich',
        })
        onSkillsChange?.(agent.skills.length ? agent.skills : undefined)
      } catch {
        onChange({ ...value, choice: 'default', namedBody: '' })
        onSkillsChange?.(undefined)
      }
    },
    [onChange, onSkillsChange, value],
  )

  const selectedEntry = entries?.find((e) => e.id === selectedId)
  const label = selectedId === null ? 'Default' : (selectedEntry?.name ?? selectedId)
  const TriggerIconComp = agentIcon(selectedEntry?.icon ?? null)

  return (
    <SelectPrimitive.Root
      value={
        selectedId === null
          ? 'default'
          : `named:${AGENT_CHOICE_PREFIX}${selectedId}`
      }
      onValueChange={handleValueChange}
      onOpenChange={handleOpenChange}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        aria-label="agent"
        className={cn(
          'inline-flex w-full items-center justify-between gap-x-2 rounded-sm border border-transparent bg-bg px-3 h-9 text-ink font-sans text-[13px] hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-active transition-colors',
          disabled && 'opacity-40 pointer-events-none',
          className,
        )}
      >
        <span className="inline-flex items-center gap-2 min-w-0">
          <TriggerIconComp size={16} className="text-ink-faint" aria-hidden />
          <span className="truncate max-w-[14rem]">
            <SelectPrimitive.Value>{label}</SelectPrimitive.Value>
          </span>
        </span>
        <SelectPrimitive.Icon asChild>
          <ChevronDown size={16} aria-hidden />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>

      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          className={cn(
            'z-50 min-w-[var(--radix-select-trigger-width)] max-w-[var(--radix-select-content-available-width)] overflow-hidden rounded-md border border-rule-2 bg-panel-raised text-ink font-sans text-[13px] shadow-floating',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
          )}
        >
          <SelectPrimitive.Viewport className="p-1">
            <AgentItem
              value="default"
              label="Default"
              description="No agent — the provider's built-in prompt"
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
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}
