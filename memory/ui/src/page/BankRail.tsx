import { useState } from 'react'
import { Button, Input } from '@iii-dev/console-ui'
import { Plus } from './icons'
import type { MemoryBank } from './memory-data'

/**
 * Left rail: the bank list (first-class named memory scopes) plus an
 * inline create form. Selecting a bank scopes the memories/rules/recall
 * panels; sessions pick their bank via session metadata `memory_bank`.
 */

interface BankRailProps {
  banks: MemoryBank[]
  selected: string | null
  onSelect: (bank: string) => void
  onCreate: (name: string) => Promise<boolean>
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
    <aside className="mem-rail">
      <div className="mem-rail-head">
        <span className="mem-rail-caption">banks</span>
      </div>
      <div className="mem-rail-list">
        {banks.map((bank) => (
          <button
            key={bank.name}
            type="button"
            onClick={() => onSelect(bank.name)}
            className={`mem-rail-item${selected === bank.name ? ' active' : ''}`}
          >
            <span className="mem-rail-name">{bank.name}</span>
            <span className="mem-rail-meta">
              {bank.memories} memories · {bank.pinned} pinned · {bank.rules}{' '}
              rules
            </span>
          </button>
        ))}
      </div>
      <form
        className="mem-rail-form"
        onSubmit={(e) => {
          e.preventDefault()
          if (!valid || creating) return
          void onCreate(draft).then((ok) => {
            if (ok) setDraft('')
          })
        }}
      >
        <Input
          value={draft}
          onChange={setDraft}
          placeholder="new bank"
          aria-label="new bank name"
          className="mem-rail-input"
        />
        <Button
          type="submit"
          variant="ghost"
          size="icon"
          disabled={!valid || creating}
          aria-label="create bank"
        >
          <Plus size={14} aria-hidden />
        </Button>
      </form>
    </aside>
  )
}
