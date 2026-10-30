// Group-by picker for the TRACES tab.
//
// Grouping accepts any span attribute key. A popover with a search field
// over two lists — the presets (message, session, function) and the keys
// observed on the loaded traces — plus a "group by <typed key>" row that
// keeps arbitrary attribute entry available. Keyboard: arrows move the
// highlight, Enter commits, Escape closes; the list is an ARIA listbox the
// search input controls.

import { Check, ChevronDown, Layers, Search } from 'lucide-react'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import { cn } from '@/lib/utils'
import type { GroupByOption } from '../lib/groupTraces'

const PRESETS: Array<{ value: GroupByOption; label: string }> = [
  { value: 'none', label: 'No grouping' },
  { value: 'iii.message.id', label: 'Message' },
  { value: 'iii.session.id', label: 'Session' },
  { value: 'iii.function.id', label: 'Function' },
]

const PRESET_VALUES = new Set<string>(PRESETS.map((preset) => preset.value))

interface Option {
  value: GroupByOption
  label: string
  /** quiet secondary text (the raw key for a preset, "observed" otherwise) */
  detail?: string
  group: 'presets' | 'observed' | 'create'
}

interface GroupByPickerProps {
  value: GroupByOption
  onChange: (next: GroupByOption) => void
  /** Attribute keys observed on loaded traces, iii.* first. */
  suggestions: string[]
}

export function GroupByPicker({
  value,
  onChange,
  suggestions,
}: GroupByPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const listId = useId()

  const observed = useMemo(
    () =>
      Array.from(
        new Set([
          ...(value !== 'none' && !PRESET_VALUES.has(value) ? [value] : []),
          ...suggestions.filter((key) => !PRESET_VALUES.has(key)),
        ]),
      ).slice(0, 100),
    [value, suggestions],
  )

  const options = useMemo<Option[]>(() => {
    const q = query.trim().toLowerCase()
    const matches = (text: string) => !q || text.toLowerCase().includes(q)
    const out: Option[] = []
    for (const preset of PRESETS) {
      if (matches(preset.label) || matches(preset.value)) {
        out.push({
          value: preset.value,
          label: preset.label,
          detail: preset.value === 'none' ? undefined : preset.value,
          group: 'presets',
        })
      }
    }
    for (const key of observed) {
      if (matches(key)) {
        out.push({
          value: key,
          label: key,
          detail: 'observed',
          group: 'observed',
        })
      }
    }
    const typed = query.trim()
    const exact = out.some(
      (o) =>
        o.value.toLowerCase() === typed.toLowerCase() ||
        o.label.toLowerCase() === typed.toLowerCase(),
    )
    if (typed && !exact) {
      out.push({ value: typed, label: typed, group: 'create' })
    }
    return out
  }, [query, observed])

  // Keep the highlight on something real as the list changes under it.
  useEffect(() => {
    if (!open) return
    setActive((i) => Math.min(Math.max(0, i), Math.max(0, options.length - 1)))
  }, [options.length, open])

  useEffect(() => {
    if (!open) return
    document
      .getElementById(`${listId}-option-${active}`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [active, open, listId])

  const commit = (option: Option | undefined) => {
    if (!option) return
    onChange(option.value)
    setOpen(false)
  }

  const handleOpenChange = (next: boolean) => {
    setOpen(next)
    if (next) {
      setQuery('')
      const idx = options.findIndex((o) => o.value === value)
      setActive(idx === -1 ? 0 : idx)
    }
  }

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      if (options.length === 0) return
      const delta = e.key === 'ArrowDown' ? 1 : -1
      setActive((i) => (i + delta + options.length) % options.length)
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault()
      setActive(e.key === 'Home' ? 0 : Math.max(0, options.length - 1))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      commit(options[active])
    }
  }

  const current = PRESETS.find((p) => p.value === value)
  const grouping = value !== 'none'
  const triggerLabel = grouping ? (current?.label ?? value) : 'Group by'

  let lastGroup: Option['group'] | null = null

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger
        render={
          // A quiet chip beside the search: ghost until a grouping is on,
          // then the soft fill names it. Secondary to the search and the
          // Filters trigger, which keep the full-height outline.
          <Button
            type="button"
            variant={grouping ? 'secondary' : 'ghost'}
            size="sm"
            aria-label="group traces by"
            aria-haspopup="listbox"
            className={cn(
              'max-w-[14rem] rounded-md',
              !grouping && 'text-muted-foreground',
            )}
          />
        }
      >
        <Layers data-icon="inline-start" className="size-4" />
        <span className="min-w-0 truncate">
          {grouping ? (
            <>
              <span className="text-muted-foreground">Group: </span>
              {triggerLabel}
            </>
          ) : (
            triggerLabel
          )}
        </span>
        <ChevronDown
          data-icon="inline-end"
          className="size-4 text-muted-foreground"
        />
      </PopoverTrigger>
      <PopoverContent
        align="start"
        collisionPadding={8}
        initialFocus={inputRef}
        className="w-72 gap-0 rounded-lg p-0"
      >
        <div className="relative border-b border-border">
          <Search
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden
          />
          <input
            ref={inputRef}
            role="combobox"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setActive(0)
            }}
            onKeyDown={onKeyDown}
            placeholder="Search attributes…"
            aria-label="search attribute keys"
            aria-autocomplete="list"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={
              options.length ? `${listId}-option-${active}` : undefined
            }
            className="h-9 w-full bg-transparent pr-3 pl-8 text-sm text-foreground outline-none placeholder:text-muted-foreground"
          />
        </div>
        <div
          id={listId}
          role="listbox"
          aria-label="group traces by"
          className="max-h-72 overflow-y-auto overscroll-contain p-1"
        >
          {options.length === 0 ? (
            <div className="px-2 py-6 text-center text-sm text-muted-foreground">
              Type an attribute key
            </div>
          ) : null}
          {options.map((option, index) => {
            const heading = option.group !== lastGroup ? option.group : null
            lastGroup = option.group
            const selected = option.value === value
            const highlighted = index === active
            return (
              <div key={`${option.group}:${option.value}`}>
                {heading === 'presets' ? (
                  <div className="px-2 pt-1.5 pb-1 text-xs font-medium text-muted-foreground">
                    Presets
                  </div>
                ) : heading === 'observed' ? (
                  <div className="px-2 pt-1.5 pb-1 text-xs font-medium text-muted-foreground">
                    Observed attributes
                  </div>
                ) : null}
                {/* biome-ignore lint/a11y/useFocusableInteractive: the combobox input owns focus; options are aria-activedescendant targets */}
                <div
                  id={`${listId}-option-${index}`}
                  role="option"
                  aria-selected={selected}
                  data-highlighted={highlighted || undefined}
                  onPointerDown={(e) => e.preventDefault()}
                  onPointerMove={() => setActive(index)}
                  onClick={() => commit(option)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault()
                      commit(option)
                    }
                  }}
                  className={cn(
                    'flex h-8 cursor-pointer items-center gap-2 rounded-md px-2 text-sm text-foreground',
                    highlighted && 'bg-muted',
                  )}
                >
                  <span
                    className={cn(
                      'min-w-0 flex-1 truncate',
                      option.group === 'observed' && 'font-mono',
                    )}
                  >
                    {option.group === 'create' ? (
                      <>
                        <span className="text-muted-foreground">Group by </span>
                        <span className="font-mono">“{option.label}”</span>
                      </>
                    ) : (
                      option.label
                    )}
                  </span>
                  {option.detail ? (
                    <span className="shrink-0 truncate font-mono text-xs text-muted-foreground">
                      {option.detail}
                    </span>
                  ) : null}
                  {selected ? (
                    <Check
                      className="size-4 shrink-0 text-foreground"
                      aria-hidden
                    />
                  ) : null}
                </div>
              </div>
            )
          })}
        </div>
      </PopoverContent>
    </Popover>
  )
}
