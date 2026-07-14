import { Pencil, Pin, PinOff, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import type { MemoryFact } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * The selected bank's facts, newest first. Every row is editable in place:
 * pin (protects from every automatic path), edit text (bumps a revision;
 * the old one stays in the on-disk log), delete (tombstone — recoverable
 * via the superseded view). A save form on top covers "remember this".
 */

interface FactsPanelProps {
  facts: MemoryFact[]
  total: number
  includeSuperseded: boolean
  onToggleSuperseded: (next: boolean) => void
  onSave: (text: string) => void
  onPin: (fact: MemoryFact) => void
  onEdit: (fact: MemoryFact, text: string) => void
  onDelete: (fact: MemoryFact) => void
  busy: boolean
}

export function FactsPanel({
  facts,
  total,
  includeSuperseded,
  onToggleSuperseded,
  onSave,
  onPin,
  onEdit,
  onDelete,
  busy,
}: FactsPanelProps) {
  const [draft, setDraft] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editText, setEditText] = useState('')

  return (
    <div className="flex flex-col gap-3">
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          const text = draft.trim()
          if (text.length < 3 || busy) return
          onSave(text)
          setDraft('')
        }}
      >
        <Input
          value={draft}
          onChange={setDraft}
          preserveCase
          placeholder="remember something..."
          aria-label="new fact"
          className="flex-1"
        />
        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={draft.trim().length < 3 || busy}
        >
          save
        </Button>
      </form>

      <div className="flex items-center justify-between">
        <span className="font-mono text-[11px] lowercase text-ink-faint">
          {total} facts
        </span>
        <label className="flex items-center gap-1.5 font-mono text-[11px] lowercase text-ink-faint cursor-pointer">
          <input
            type="checkbox"
            checked={includeSuperseded}
            onChange={(e) => onToggleSuperseded(e.target.checked)}
            className="accent-[var(--color-accent,#ff5a1f)]"
          />
          show history
        </label>
      </div>

      {facts.length === 0 ? (
        <EmptyState
          title="no facts in this bank yet"
          description="facts arrive automatically after each completed turn, or save one above. sessions pick this bank via session metadata memory_bank."
        />
      ) : (
        <ul className="border border-rule divide-y divide-rule-2">
          {facts.map((fact) => {
            const superseded = fact.invalid_at != null
            const editing = editingId === fact.id
            return (
              <li
                key={fact.id}
                className={cn(
                  'px-3 py-2 flex flex-col gap-1.5',
                  fact.pinned && 'border-l-2 border-l-accent',
                  superseded && 'opacity-50',
                )}
              >
                {editing ? (
                  <form
                    className="flex items-center gap-2"
                    onSubmit={(e) => {
                      e.preventDefault()
                      const text = editText.trim()
                      if (text.length < 3) return
                      onEdit(fact, text)
                      setEditingId(null)
                    }}
                  >
                    <Input
                      value={editText}
                      onChange={setEditText}
                      preserveCase
                      aria-label="edit fact"
                      className="flex-1"
                    />
                    <Button type="submit" variant="primary" size="sm">
                      save
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => setEditingId(null)}
                    >
                      cancel
                    </Button>
                  </form>
                ) : (
                  <p className="font-mono text-[13px] text-ink leading-snug">
                    {fact.text}
                  </p>
                )}
                <div className="flex items-center gap-2 flex-wrap">
                  {fact.entities.map((entity) => (
                    <Badge key={entity}>{entity}</Badge>
                  ))}
                  <span className="font-mono text-[10px] lowercase text-ink-ghost">
                    {fact.confidence}
                    {fact.corroboration > 0 &&
                      ` · seen ×${fact.corroboration + 1}`}
                    {superseded && ' · superseded'}
                  </span>
                  <span className="flex-1" />
                  <Button
                    variant="icon"
                    size="icon"
                    onClick={() => onPin(fact)}
                    disabled={busy || superseded}
                    aria-label={fact.pinned ? 'unpin fact' : 'pin fact'}
                    title={
                      fact.pinned
                        ? 'unpin (allows automatic consolidation again)'
                        : 'pin (untouchable by every automatic path)'
                    }
                  >
                    {fact.pinned ? (
                      <PinOff className="w-3.5 h-3.5" aria-hidden />
                    ) : (
                      <Pin className="w-3.5 h-3.5" aria-hidden />
                    )}
                  </Button>
                  <Button
                    variant="icon"
                    size="icon"
                    onClick={() => {
                      setEditingId(fact.id)
                      setEditText(fact.text)
                    }}
                    disabled={busy || superseded}
                    aria-label="edit fact"
                  >
                    <Pencil className="w-3.5 h-3.5" aria-hidden />
                  </Button>
                  <Button
                    variant="icon"
                    size="icon"
                    onClick={() => onDelete(fact)}
                    disabled={busy || superseded}
                    aria-label="delete fact"
                    title="tombstone (leaves recall; stays on disk under show history)"
                  >
                    <Trash2 className="w-3.5 h-3.5" aria-hidden />
                  </Button>
                </div>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
