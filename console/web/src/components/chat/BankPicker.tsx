import { useEffect, useState } from 'react'
import { Select } from '@/components/ui/Select'
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

export function BankPicker({ value, onChange, disabled }: BankPickerProps) {
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

  const known = banks.some((b) => b.name === value)
  const options = [
    {
      value: AUTO,
      label: 'memory: auto',
      title: "use the memory worker's default bank",
    },
    ...banks.map((b) => ({
      value: b.name,
      label: `memory: ${b.name}`,
      title: `${b.memories} memories · ${b.rules} rules — rules + recalled memories from this bank feed every turn`,
    })),
    // A bank set elsewhere (CLI, another client) that we haven't listed.
    ...(value && !known
      ? [
          {
            value,
            label: `memory: ${value}`,
            title: 'set outside this console',
          },
        ]
      : []),
  ]

  return (
    <Select<string>
      value={value ?? AUTO}
      onChange={(next) => onChange(next === AUTO ? null : next)}
      disabled={disabled}
      aria-label="memory bank"
      className="min-w-[120px] shrink-0"
      options={options}
    />
  )
}
