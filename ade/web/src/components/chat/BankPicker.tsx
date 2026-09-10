import { useEffect, useState } from 'react'
import { Select } from '@/components/ui/Select'
import { SheetOptionList } from '@/components/ui/SheetNavigation'
import { listBanks, type MemoryBank } from '@/lib/memory'

/**
 * In-chat memory bank picker — the "which memory am I using" control.
 * Writing blogs? Pick the blog bank and its style rules + memories feed
 * every turn. Switch to coding and a different memory applies; contexts
 * never bleed. Selecting `auto` defers to the memory worker's configured
 * default bank. Lives next to the composer as one compact dropdown and
 * commits through session metadata (`memory_bank`), so it applies to the
 * NEXT turn immediately, mid-conversation switches included.
 */

interface BankPickerProps {
  /** Bank for THIS conversation; null = the worker's default bank. */
  value: string | null
  onChange: (next: string | null) => void
  disabled?: boolean
}

const AUTO = '(auto)'

function useBankOptions(value: string | null, conciseLabels: boolean) {
  const [banks, setBanks] = useState<MemoryBank[]>([])

  useEffect(() => {
    let cancelled = false
    void listBanks()
      .then((next) => {
        if (!cancelled) setBanks(next)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const known = banks.some((bank) => bank.name === value)
  return [
    {
      value: AUTO,
      label: conciseLabels ? 'Auto' : 'memory: auto',
      title: "use the memory worker's default bank",
    },
    ...banks.map((bank) => ({
      value: bank.name,
      label: conciseLabels ? bank.name : `memory: ${bank.name}`,
      title: `${bank.memories} memories · ${bank.rules} rules — rules + recalled memories from this bank feed every turn`,
    })),
    ...(value && !known
      ? [
          {
            value,
            label: conciseLabels ? value : `memory: ${value}`,
            title: 'set outside this console',
          },
        ]
      : []),
  ]
}

export function BankPicker({ value, onChange, disabled }: BankPickerProps) {
  const options = useBankOptions(value, false)

  return (
    <Select<string>
      value={value ?? AUTO}
      onChange={(next) => onChange(next === AUTO ? null : next)}
      disabled={disabled}
      aria-label="memory bank"
      className="min-w-[96px] max-w-[160px]"
      options={options}
    />
  )
}

interface BankPickerPanelProps {
  value: string | null
  onChange: (next: string | null) => void
  disabled?: boolean
  className?: string
}

/** Inline memory-bank choices for an existing navigable sheet. */
export function BankPickerPanel({
  value,
  onChange,
  disabled,
  className,
}: BankPickerPanelProps) {
  const options = useBankOptions(value, true)
  return (
    <SheetOptionList
      value={value ?? AUTO}
      disabled={disabled}
      className={className}
      onChange={(next) => onChange(next === AUTO ? null : next)}
      options={options.map((option) => ({
        value: option.value,
        label: option.label,
        description: option.title,
      }))}
    />
  )
}
