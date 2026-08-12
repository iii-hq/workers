import * as SelectPrimitive from '@radix-ui/react-select'
import { Check, ChevronDown, ScrollText } from 'lucide-react'
import { useCallback, useState } from 'react'
import {
  getPrompt,
  listPrompts,
  type PromptEntry,
} from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import {
  choiceToValue,
  type SystemPromptState,
  valueToChoice,
} from './system-prompt-selection'

interface SystemPromptPickerProps {
  value: SystemPromptState
  onChange: (next: SystemPromptState) => void
  disabled?: boolean
  /**
   * Offer the `custom…` free-text row. The new-session screen turns it off:
   * authoring a prompt lives in the iii-directory UI's system-prompts tab,
   * not in the chat.
   */
  allowCustom?: boolean
  className?: string
}

/** One row of the select — ModePicker's item chrome, verbatim. */
function PromptItem({
  value,
  label,
  description,
}: {
  value: string
  label: string
  description?: string
}) {
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
        <Check size={12} aria-hidden />
      </SelectPrimitive.ItemIndicator>
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

export function SystemPromptPicker({
  value,
  onChange,
  disabled,
  allowCustom = true,
  className,
}: SystemPromptPickerProps) {
  const [entries, setEntries] = useState<PromptEntry[] | null>(null)

  /* Fetch on EVERY open (keeping the last list while loading): a prompt
     added on disk or authored in the directory UI shows up on the next
     open with no cache-invalidation plumbing. A failed load degrades to
     default + custom only (directory worker absent ≠ broken chat). */
  const handleOpenChange = useCallback(async (open: boolean) => {
    if (!open) return
    try {
      setEntries(await listPrompts(await getIiiClient()))
    } catch {
      setEntries((prev) => prev ?? [])
    }
  }, [])

  const handleValueChange = useCallback(
    async (v: string) => {
      const choice = valueToChoice(v)
      if (typeof choice === 'object') {
        /* Resolve the body at selection time (spec decision #4). */
        try {
          const p = await getPrompt(await getIiiClient(), choice.named)
          onChange({ ...value, choice, namedBody: p.body })
        } catch {
          onChange({ ...value, choice: 'default', namedBody: '' })
        }
      } else {
        onChange({ ...value, choice })
      }
    },
    [onChange, value],
  )

  const label =
    value.choice === 'default'
      ? 'default'
      : value.choice === 'custom'
        ? 'custom'
        : value.choice.named

  return (
    <SelectPrimitive.Root
      value={choiceToValue(value.choice)}
      onValueChange={handleValueChange}
      onOpenChange={handleOpenChange}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        aria-label="system prompt"
        className={cn(
          'inline-flex w-full items-center justify-between gap-x-2 rounded-sm border border-transparent bg-bg px-3 h-9 text-ink font-mono text-[13px] lowercase hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-active transition-colors',
          disabled && 'opacity-40 pointer-events-none',
          className,
        )}
      >
        <span className="inline-flex items-center gap-2 min-w-0">
          <ScrollText size={14} className="text-ink-faint" aria-hidden />
          {/* Radix strips `className` off Select.Value, so the truncation
              lives on this wrapper (a flex item, hence blockified). */}
          <span className="truncate max-w-[10rem]">
            <SelectPrimitive.Value>{label}</SelectPrimitive.Value>
          </span>
        </span>

        <SelectPrimitive.Icon asChild>
          <ChevronDown size={12} aria-hidden />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>

      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          className={cn(
            'z-50 min-w-[var(--radix-select-trigger-width)] max-w-[var(--radix-select-content-available-width)] overflow-hidden rounded-md border border-rule-2 bg-panel-raised text-ink font-mono text-[13px] lowercase shadow-floating',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
          )}
        >
          <SelectPrimitive.Viewport className="p-1">
            <PromptItem
              value="default"
              label="default"
              description="use the provider's built-in prompt"
            />
            {(entries ?? []).map((e) => (
              <PromptItem
                key={e.name}
                value={choiceToValue({ named: e.name })}
                label={e.name}
                description={e.description}
              />
            ))}
            {allowCustom ? <PromptItem value="custom" label="custom…" /> : null}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}

/** Explicit two-state choice shown whenever the prompt isn't the default. */
export function StrategyToggle({
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
  if (value.choice === 'default') return null
  return (
    <fieldset
      disabled={disabled}
      className={cn(
        'inline-grid min-w-0 grid-cols-2 rounded-sm border-0 bg-bg p-[2px] gap-[2px]',
        className,
      )}
    >
      <legend className="sr-only">apply system prompt as</legend>
      {(['enrich', 'override'] as const).map((strategy) => {
        const active = value.strategy === strategy
        const label = strategy === 'enrich' ? 'enrich' : 'replace'
        const detail =
          strategy === 'enrich'
            ? 'add this prompt to the built-in prompt'
            : 'use this prompt instead of the built-in prompt'
        return (
          <button
            key={strategy}
            type="button"
            aria-label={`${label}: ${detail}`}
            aria-pressed={active}
            title={detail}
            onClick={() => onChange({ ...value, strategy })}
            className={cn(
              'relative inline-flex h-8 min-w-[82px] items-center justify-center rounded-xs px-2 font-mono text-[13px] font-medium lowercase transition-colors focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40',
              active
                ? 'bg-accent-muted text-ink'
                : 'bg-transparent text-ink-faint hover:bg-surface-hover hover:text-ink',
            )}
          >
            <Check
              size={12}
              aria-hidden
              className={cn(
                'absolute left-2 shrink-0 text-accent transition-opacity',
                active ? 'opacity-100' : 'opacity-0',
              )}
            />
            {label}
          </button>
        )
      })}
    </fieldset>
  )
}
