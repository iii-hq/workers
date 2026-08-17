/**
 * Navigation column: the bank list (first-class named memory scopes) plus
 * an inline create form. Selecting a bank scopes the rules/memories/graph/
 * preview workspace; sessions pick their bank via session metadata
 * `memory_bank`. In the narrow drill-in flow this list is the first pane.
 */

import { useState } from 'react'
import { Button, Input } from '@iii-dev/console-ui'
import { Plus } from './icons'
import type { MemoryBank } from './memory-data'

interface BankRailProps {
  banks: MemoryBank[]
  selected: string | null
  /** True while the page-level refresh is in flight (skeleton on first load). */
  loading: boolean
  onSelect: (bank: string) => void
  onCreate: (name: string) => Promise<boolean>
  creating: boolean
}

export function BankRail({
  banks,
  selected,
  loading,
  onSelect,
  onCreate,
  creating,
}: BankRailProps) {
  const [draft, setDraft] = useState('')
  const valid = /^[a-z0-9][a-z0-9_-]{0,63}$/.test(draft)
  const initialLoad = loading && banks.length === 0

  return (
    <aside className="mem-ui-rail" aria-label="bank list">
      <header className="mem-ui-col-head">
        <span className="label">banks</span>
        <span className="spacer" />
        {initialLoad ? null : <span className="count">{banks.length}</span>}
      </header>
      <div className="mem-ui-rail-scroll">
        {initialLoad ? (
          <div className="mem-ui-rail-skel" aria-hidden>
            {[
              [60, 90],
              [40, 80],
              [70, 90],
            ].map(([a, b]) => (
              <div key={`${a}-${b}`} className="mem-ui-skel-row">
                <span className={`bar w${a}`} />
                <span className={`bar w${b}`} />
              </div>
            ))}
          </div>
        ) : banks.length === 0 ? (
          <div className="mem-ui-rail-empty">
            <p>No banks yet.</p>
            <p className="dim">
              Create one below, or just chat: the default bank materializes
              when the first memory is saved.
            </p>
          </div>
        ) : (
          <ul className="mem-ui-nav-list">
            {banks.map((bank) => (
              <li key={bank.name}>
                <button
                  type="button"
                  className={`mem-ui-nav-row${selected === bank.name ? ' active' : ''}`}
                  aria-current={selected === bank.name ? 'true' : undefined}
                  onClick={() => onSelect(bank.name)}
                >
                  <span className="name">{bank.name}</span>
                  <span className="fine">
                    {bank.memories} memories · {bank.pinned} pinned ·{' '}
                    {bank.rules} rules
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <form
        className="mem-ui-rail-form"
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
          className="mem-ui-rail-input"
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
