import * as SelectPrimitive from '@radix-ui/react-select'
import { Check, ChevronDown, ScrollText } from 'lucide-react'
import { useCallback, useEffect, useId, useState } from 'react'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { Select } from '@/components/ui/Select'
import { useMediaQuery } from '@/hooks/use-media-query'
import {
  getPrompt,
  listPrompts,
  type PromptEntry,
} from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'
import {
  choiceToValue,
  type PromptStrategy,
  type SystemPromptState,
  valueToChoice,
} from './system-prompt-selection'

interface SystemPromptPickerProps {
  value: SystemPromptState
  onChange: (next: SystemPromptState) => void
  disabled?: boolean
  /**
   * Offer the `custom…` free-text row. Authoring a prompt otherwise lives in
   * the iii-directory UI's system-prompts tab.
   */
  allowCustom?: boolean
  /** Text-only treatment used alongside the empty-state project sentence. */
  appearance?: 'default' | 'inline'
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
        <Check size={16} aria-hidden />
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

interface SystemPromptPickerPanelProps {
  value: SystemPromptState
  entries: PromptEntry[] | null
  allowCustom: boolean
  disabled?: boolean
  onSelect: (value: string) => void
}

/** Mobile option list for the shared sheet surface. */
export function SystemPromptPickerPanel({
  value,
  entries,
  allowCustom,
  disabled,
  onSelect,
}: SystemPromptPickerPanelProps) {
  const name = useId()
  const selectedValue = choiceToValue(value.choice)
  const options = [
    {
      value: 'default',
      label: 'Default',
      description: "Use the provider's built-in prompt",
    },
    ...(entries ?? []).map((entry) => ({
      value: choiceToValue({ named: entry.name }),
      label: entry.name,
      description: entry.description,
    })),
    ...(allowCustom
      ? [{ value: 'custom', label: 'Custom…', description: undefined }]
      : []),
  ]

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-1">
      <fieldset
        disabled={disabled}
        className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge"
      >
        <legend className="sr-only">System prompt</legend>
        {options.map((option) => {
          const selected = option.value === selectedValue
          return (
            <label
              key={option.value}
              className={cn(
                'flex min-h-14 w-full min-w-0 cursor-pointer items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-inset has-[:focus-visible]:ring-rule-focus',
                selected && 'bg-surface-selected',
                disabled && 'pointer-events-none opacity-40',
              )}
            >
              <input
                type="radio"
                name={name}
                value={option.value}
                checked={selected}
                disabled={disabled}
                onChange={() => onSelect(option.value)}
                className="sr-only"
              />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate font-medium">{option.label}</span>
                {option.description ? (
                  <span className="truncate text-sm leading-relaxed text-ink-faint">
                    {option.description}
                  </span>
                ) : null}
              </span>
              {selected ? (
                <Check className="size-5 shrink-0 text-ink" aria-hidden />
              ) : null}
            </label>
          )
        })}
        {entries === null ? (
          <div className="px-3 py-3 font-sans text-sm text-ink-faint">
            Loading saved prompts…
          </div>
        ) : null}
      </fieldset>
    </div>
  )
}

export function SystemPromptPicker({
  value,
  onChange,
  disabled,
  allowCustom = true,
  appearance = 'default',
  className,
}: SystemPromptPickerProps) {
  const [open, setOpen] = useState(false)
  const [entries, setEntries] = useState<PromptEntry[] | null>(null)
  const mobileSheet = useMediaQuery('(max-width: 767px)')

  /* Fetch on EVERY open (keeping the last list while loading): a prompt
     added on disk or authored in the directory UI shows up on the next
     open with no cache-invalidation plumbing. A failed load degrades to
     default + custom only (directory worker absent ≠ broken chat). */
  const handleOpenChange = useCallback(async (nextOpen: boolean) => {
    setOpen(nextOpen)
    if (!nextOpen) return
    try {
      setEntries(await listPrompts(await getIiiClient()))
    } catch {
      setEntries((prev) => prev ?? [])
    }
  }, [])

  useEffect(() => {
    if (disabled && open) setOpen(false)
  }, [disabled, open])

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
      ? 'Default'
      : value.choice === 'custom'
        ? 'Custom'
        : value.choice.named

  const triggerClassName = cn(
    appearance === 'inline'
      ? 'relative inline-flex min-h-5.5 min-w-0 items-center border-dashed border-b border-ink-faint/50 px-0.5 font-sans text-base font-medium text-ink hover:border-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:border-ink sm:text-[0.8125rem]'
      : 'inline-flex h-9 w-full items-center justify-between gap-x-2 rounded-sm border border-transparent bg-bg px-3 font-sans text-[0.8125rem] text-ink transition-colors hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface-active',
    disabled && 'pointer-events-none opacity-40',
    className,
  )

  const triggerContent = (
    <>
      <span
        className={cn(
          'inline-flex min-w-0 items-center',
          appearance === 'default' && 'gap-2',
        )}
      >
        {appearance === 'default' ? (
          <ScrollText
            size={16}
            className="shrink-0 text-ink-faint"
            aria-hidden
          />
        ) : null}
        <span className="max-w-40 truncate sm:max-w-44">{label}</span>
      </span>

      {appearance === 'default' ? (
        <ChevronDown className="size-4 shrink-0" aria-hidden />
      ) : (
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
      )}
    </>
  )

  if (mobileSheet) {
    return (
      <>
        <button
          type="button"
          aria-label={`system prompt, current prompt: ${label}`}
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
          <BottomSheetContent
            heading="System prompt"
            closeLabel="Close system prompt picker"
          >
            <SystemPromptPickerPanel
              value={value}
              entries={entries}
              allowCustom={allowCustom}
              disabled={disabled}
              onSelect={(next) => {
                setOpen(false)
                void handleValueChange(next)
              }}
            />
          </BottomSheetContent>
        </BottomSheet>
      </>
    )
  }

  return (
    <SelectPrimitive.Root
      value={choiceToValue(value.choice)}
      onValueChange={handleValueChange}
      onOpenChange={handleOpenChange}
      open={open}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        aria-label={`system prompt, current prompt: ${label}`}
        className={triggerClassName}
      >
        <span
          className={cn(
            'inline-flex min-w-0 items-center',
            appearance === 'default' && 'gap-2',
          )}
        >
          {appearance === 'default' ? (
            <ScrollText
              size={16}
              className="shrink-0 text-ink-faint"
              aria-hidden
            />
          ) : null}
          {/* Radix strips `className` off Select.Value, so the truncation
              lives on this wrapper (a flex item, hence blockified). */}
          <span className="max-w-40 truncate sm:max-w-44">{label}</span>
        </span>

        {appearance === 'default' ? (
          <SelectPrimitive.Icon asChild>
            <ChevronDown className="size-4 shrink-0" aria-hidden />
          </SelectPrimitive.Icon>
        ) : (
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
        )}
      </SelectPrimitive.Trigger>

      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          className={cn(
            'iii-ui-motion-dropdown z-50 min-w-[var(--radix-select-trigger-width)] max-w-[var(--radix-select-content-available-width)] overflow-hidden rounded-md border border-rule-2 bg-panel-raised text-ink font-sans text-[13px] shadow-floating',
          )}
        >
          <SelectPrimitive.Viewport className="p-1">
            <PromptItem
              value="default"
              label="Default"
              description="Use the provider's built-in prompt"
            />
            {(entries ?? []).map((e) => (
              <PromptItem
                key={e.name}
                value={choiceToValue({ named: e.name })}
                label={e.name}
                description={e.description}
              />
            ))}
            {allowCustom ? <PromptItem value="custom" label="Custom…" /> : null}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}

const PROMPT_STRATEGIES: Array<{
  value: PromptStrategy
  label: string
  description: string
}> = [
  {
    value: 'enrich',
    label: 'Extending',
    description: 'Add this prompt to the built-in prompt',
  },
  {
    value: 'override',
    label: 'Overriding',
    description: 'Use this prompt instead of the built-in prompt',
  },
]

/** Strategy choice shown whenever the prompt isn't the default. */
export function StrategyToggle({
  value,
  onChange,
  disabled,
  appearance = 'default',
  className,
}: {
  value: SystemPromptState
  onChange: (next: SystemPromptState) => void
  disabled?: boolean
  appearance?: 'default' | 'inline'
  className?: string
}) {
  if (value.choice === 'default') return null
  return (
    <Select<PromptStrategy>
      value={value.strategy}
      onChange={(strategy) => onChange({ ...value, strategy })}
      options={PROMPT_STRATEGIES}
      disabled={disabled}
      appearance={appearance}
      aria-label="system prompt strategy"
      sheetTitle="System prompt strategy"
      sheetDescription="Choose how this prompt changes the built-in prompt."
      className={className}
    />
  )
}
