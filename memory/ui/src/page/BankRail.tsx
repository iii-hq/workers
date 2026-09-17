/**
 * Navigation column: the bank list (first-class named memory scopes) plus
 * an inline create form. Selecting a bank scopes the rules/memories/graph/
 * preview workspace; sessions pick their bank via session metadata
 * `memory_bank`. In the narrow drill-in flow this list is the first pane.
 */

import {
  Button,
  EmptyState,
  Input,
  List,
  ListItem,
  Skeleton,
} from '@iii-dev/console-ui'
import { Plus } from 'lucide-react'
import { useState } from 'react'
import type { MemoryBank } from './memory-data'

interface BankRailProps {
  banks: MemoryBank[]
  selected: string | null
  /** True while the page-level refresh is in flight (skeleton on first load). */
  loading: boolean
  onSelect: (bank: string) => void
  onCreate: (name: string) => Promise<boolean>
  creating: boolean
  /** Focus the first bank row when the page opens with no bank selected. */
  autofocusFirst?: boolean
}

export function BankRail({
  banks,
  selected,
  loading,
  onSelect,
  onCreate,
  creating,
  autofocusFirst,
}: BankRailProps) {
  const [draft, setDraft] = useState('')
  const valid = /^[a-z0-9][a-z0-9_-]{0,63}$/.test(draft)
  const initialLoad = loading && banks.length === 0

  return (
    <>
      <div className="mem-ui-rail-scroll">
        {initialLoad ? (
          <div className="mem-ui-rail-skel" aria-hidden>
            {[
              [60, 90],
              [40, 80],
              [70, 90],
            ].map(([a, b]) => (
              <div key={`${a}-${b}`} className="mem-ui-skel-row">
                <Skeleton className="mem-ui-skel" style={{ width: `${a}%` }} />
                <Skeleton className="mem-ui-skel" style={{ width: `${b}%` }} />
              </div>
            ))}
          </div>
        ) : banks.length === 0 ? (
          <EmptyState
            compact
            title="No banks yet."
            description="Create one below, or just chat: the default bank materializes when the first memory is saved."
          />
        ) : (
          <List aria-label="banks">
            {banks.map((bank, index) => (
              <ListItem
                key={bank.name}
                selected={selected === bank.name}
                aria-current={selected === bank.name ? 'true' : undefined}
                data-autofocus={autofocusFirst && index === 0 ? '' : undefined}
                onClick={() => onSelect(bank.name)}
                label={bank.name}
                description={`${bank.memories} memories · ${bank.pinned} pinned · ${bank.rules} rules`}
              />
            ))}
          </List>
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
          data-mem-new-bank-input=""
        />
        <Button
          type="submit"
          variant="ghost"
          size="icon"
          disabled={!valid || creating}
          aria-label="create bank"
        >
          <Plus size={16} aria-hidden />
        </Button>
      </form>
    </>
  )
}
