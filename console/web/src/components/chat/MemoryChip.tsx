import { useState } from 'react'
import { getFact, type MemoryFact } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * The "what fed this reply" chip on assistant messages: bank name + how
 * many facts the memory worker injected into the generate. Click to
 * expand the exact facts (fetched by id on first open) — memory acting
 * visibly inside the conversation, not just in traces.
 */

interface MemoryChipProps {
  memory: {
    bank: string
    facts: number
    factIds: string[]
    blocks?: number
    truncated?: boolean
  }
}

export function MemoryChip({ memory }: MemoryChipProps) {
  const [open, setOpen] = useState(false)
  const [facts, setFacts] = useState<MemoryFact[] | null>(null)
  const [loading, setLoading] = useState(false)

  const parts = []
  if (memory.facts > 0)
    parts.push(`${memory.facts} fact${memory.facts === 1 ? '' : 's'}`)
  if (memory.blocks)
    parts.push(`${memory.blocks} block${memory.blocks === 1 ? '' : 's'}`)
  if (parts.length === 0) return null

  const toggle = async () => {
    const next = !open
    setOpen(next)
    if (next && facts === null && memory.factIds.length > 0) {
      setLoading(true)
      try {
        const loaded = await Promise.all(
          memory.factIds.map((id) => getFact(memory.bank, id)),
        )
        setFacts(loaded.filter((f): f is MemoryFact => f !== null))
      } finally {
        setLoading(false)
      }
    }
  }

  return (
    <span className="inline-flex flex-col items-start normal-case tracking-normal">
      <button
        type="button"
        onClick={() => void toggle()}
        title="what the memory worker fed this reply — click for the exact facts"
        className={cn(
          'font-mono text-[10px] lowercase px-1.5 py-0.5 border transition-colors',
          open
            ? 'border-accent text-ink'
            : 'border-rule text-ink-faint hover:border-ink hover:text-ink',
        )}
      >
        memory: {memory.bank} · {parts.join(' · ')}
        {memory.truncated ? ' · truncated' : ''}
      </button>
      {open ? (
        <span className="mt-1 flex flex-col gap-1 border border-rule-2 bg-panel px-2 py-1.5 max-w-md">
          {loading ? (
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              loading facts…
            </span>
          ) : facts && facts.length > 0 ? (
            facts.map((fact) => (
              <span
                key={fact.id}
                className="font-mono text-[11px] text-ink leading-snug"
              >
                - {fact.text}
                {fact.pinned ? (
                  <span className="text-accent"> ·pinned</span>
                ) : null}
              </span>
            ))
          ) : (
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              {memory.factIds.length === 0
                ? 'only blocks fed this turn (always-injected markdown)'
                : 'facts no longer available (superseded or bank changed)'}
            </span>
          )}
        </span>
      ) : null}
    </span>
  )
}
