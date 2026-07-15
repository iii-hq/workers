import { Plus } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import type { MemoryBank } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * Left rail: the bank list (first-class named memory scopes) plus an
 * inline create form. Selecting a bank scopes the facts/blocks/recall
 * panels; sessions pick their bank via session metadata `memory_bank`.
 */

interface BankRailProps {
  banks: MemoryBank[]
  selected: string | null
  onSelect: (bank: string) => void
  onCreate: (name: string) => void
  creating: boolean
}

export function BankRail({
  banks,
  selected,
  onSelect,
  onCreate,
  creating,
}: BankRailProps) {
  const [draft, setDraft] = useState('')
  const valid = /^[a-z0-9][a-z0-9_-]{0,63}$/.test(draft)

  return (
    <aside className="w-52 shrink-0 border-r border-rule flex flex-col min-h-0">
      <div className="px-3 py-2 border-b border-rule-2">
        <span className="font-mono text-[11px] uppercase tracking-[0.16em] text-ink-faint font-semibold">
          banks
        </span>
      </div>
      <div className="flex-1 overflow-auto">
        {banks.map((bank) => (
          <button
            key={bank.name}
            type="button"
            onClick={() => onSelect(bank.name)}
            className={cn(
              'w-full text-left px-3 py-2 border-b border-rule-2 font-mono lowercase',
              'hover:bg-panel transition-colors',
              selected === bank.name
                ? 'border-l-2 border-l-accent bg-panel'
                : 'border-l-2 border-l-transparent',
            )}
          >
            <span className="block text-[13px] text-ink">{bank.name}</span>
            <span className="block text-[11px] text-ink-faint mt-0.5">
              {bank.facts} memories · {bank.pinned} pinned · {bank.blocks} rules
            </span>
          </button>
        ))}
      </div>
      <form
        className="p-2 border-t border-rule flex items-center gap-1.5"
        onSubmit={(e) => {
          e.preventDefault()
          if (!valid || creating) return
          onCreate(draft)
          setDraft('')
        }}
      >
        <Input
          value={draft}
          onChange={setDraft}
          placeholder="new bank"
          aria-label="new bank name"
          className="flex-1 min-w-0"
        />
        <Button
          type="submit"
          variant="ghost"
          size="icon"
          disabled={!valid || creating}
          aria-label="create bank"
        >
          <Plus className="w-3.5 h-3.5" aria-hidden />
        </Button>
      </form>
    </aside>
  )
}
