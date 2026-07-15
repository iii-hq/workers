import { useState } from 'react'
import { getMemory, type MemoryItem } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * The "what fed this reply" chip on assistant messages: bank name + how
 * many memories the memory worker injected into the generate. Click to
 * expand the exact memories (fetched by id on first open) — memory acting
 * visibly inside the conversation, not just in traces.
 */

interface MemoryChipProps {
  memory: {
    bank: string
    memories: number
    memoryIds: string[]
    rules?: number
    truncated?: boolean
    semantic?: boolean
  }
}

export function MemoryChip({ memory }: MemoryChipProps) {
  const [open, setOpen] = useState(false)
  const [memories, setFacts] = useState<MemoryItem[] | null>(null)
  const [loading, setLoading] = useState(false)

  const parts = []
  if (memory.memories > 0)
    parts.push(`${memory.memories} memor${memory.memories === 1 ? 'y' : 'ies'}`)
  if (memory.rules)
    parts.push(`${memory.rules} rule${memory.rules === 1 ? '' : 's'}`)
  if (parts.length === 0) return null

  const toggle = async () => {
    const next = !open
    setOpen(next)
    if (next && memories === null && memory.memoryIds.length > 0) {
      setLoading(true)
      try {
        const loaded = await Promise.all(
          memory.memoryIds.map((id) => getMemory(memory.bank, id)),
        )
        setFacts(loaded.filter((f): f is MemoryItem => f !== null))
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
        title="what the memory worker fed this reply — click for the exact memories"
        className={cn(
          'font-mono text-[10px] lowercase px-1.5 py-0.5 border transition-colors',
          open
            ? 'border-accent text-ink'
            : 'border-rule text-ink-faint hover:border-ink hover:text-ink',
        )}
      >
        memory: {memory.bank} · {parts.join(' · ')}
        {memory.semantic ? ' · semantic' : ''}
        {memory.truncated ? ' · truncated' : ''}
      </button>
      {open ? (
        <span className="mt-1 flex flex-col gap-1 border border-rule-2 bg-panel px-2 py-1.5 max-w-md">
          {loading ? (
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              loading memories…
            </span>
          ) : memories && memories.length > 0 ? (
            memories.map((memory) => (
              <span
                key={memory.id}
                className="font-mono text-[11px] text-ink leading-snug"
              >
                - {memory.text}
                {memory.pinned ? (
                  <span className="text-accent"> ·pinned</span>
                ) : null}
              </span>
            ))
          ) : (
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              {memory.memoryIds.length === 0
                ? 'only rules fed this turn (always-injected markdown)'
                : 'memories no longer available (superseded or bank changed)'}
            </span>
          )}
        </span>
      ) : null}
    </span>
  )
}
