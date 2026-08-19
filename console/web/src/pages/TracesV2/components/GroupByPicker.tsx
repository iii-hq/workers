// Group-by picker for the TRACES tab.
//
// Grouping accepts any span attribute key. The shared Selector provides the
// searchable/keyboard/ARIA contract; presets and observed keys are regular
// options, while `onCreate` keeps arbitrary attribute entry available.

import { Layers } from 'lucide-react'
import { Selector, type SelectorGroup } from '@/components/ui/Selector'
import type { GroupByOption } from '../lib/groupTraces'

const PRESETS: Array<{ value: GroupByOption; label: string }> = [
  { value: 'none', label: 'no grouping' },
  { value: 'iii.message.id', label: 'message' },
  { value: 'iii.session.id', label: 'session' },
  { value: 'iii.function.id', label: 'function' },
]

const PRESET_VALUES = new Set<string>(PRESETS.map((preset) => preset.value))

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
  const attributes = Array.from(
    new Set([
      ...(value !== 'none' && !PRESET_VALUES.has(value) ? [value] : []),
      ...suggestions.filter((key) => !PRESET_VALUES.has(key)),
    ]),
  ).slice(0, 100)
  const groups: SelectorGroup<GroupByOption>[] = [
    { label: 'presets', options: PRESETS },
    {
      label: 'observed attributes',
      options: attributes.map((attribute) => ({
        value: attribute,
        label: attribute,
        description: 'observed span attribute',
      })),
    },
  ]

  return (
    <Selector<GroupByOption>
      value={value}
      onChange={onChange}
      groups={groups}
      onCreate={(attribute) => onChange(attribute)}
      createOptionLabel={(attribute) => `group by “${attribute}”`}
      triggerIcon={<Layers className="size-4" />}
      aria-label="group traces by"
      placeholder="no grouping"
      searchPlaceholder="search attributes…"
      emptyMessage="type an attribute key"
      className="w-[min(18rem,70vw)]"
    />
  )
}
