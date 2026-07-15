import {
  ChevronLeft,
  ChevronRight,
  Pencil,
  Pin,
  PinOff,
  Search,
  Trash2,
  X,
} from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import { type MemoryItem, recall } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * The selected bank's memories, built for large banks: server-side pages of
 * `pageSize` (newest first) with explicit paging controls, and a search
 * box that runs `memory::recall` (the ranked scorer, not a client filter)
 * so finding one memory among 10k costs one call. Every row is editable in
 * place: pin, edit text (revision bump), tombstone delete.
 */

interface MemoriesPanelProps {
  bank: string
  memories: MemoryItem[]
  total: number
  offset: number
  pageSize: number
  onOffsetChange: (next: number) => void
  includeSuperseded: boolean
  onToggleSuperseded: (next: boolean) => void
  onSave: (text: string) => Promise<boolean>
  onPin: (memory: MemoryItem) => void
  onEdit: (memory: MemoryItem, text: string) => Promise<boolean>
  onDelete: (memory: MemoryItem) => void
  busy: boolean
}

function FactRow({
  memory,
  onPin,
  onEdit,
  onDelete,
  busy,
  score,
}: {
  memory: MemoryItem
  onPin: (memory: MemoryItem) => void
  onEdit: (memory: MemoryItem, text: string) => Promise<boolean>
  onDelete: (memory: MemoryItem) => void
  busy: boolean
  score?: number
}) {
  const [editing, setEditing] = useState(false)
  const [editText, setEditText] = useState(memory.text)
  const superseded = memory.invalid_at != null

  return (
    <li
      className={cn(
        'px-3 py-2 flex flex-col gap-1.5',
        memory.pinned && 'border-l-2 border-l-accent',
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
            void onEdit(memory, text).then((ok) => {
              if (ok) setEditing(false)
            })
          }}
        >
          <Input
            value={editText}
            onChange={setEditText}
            preserveCase
            aria-label="edit memory"
            className="flex-1"
          />
          <Button type="submit" variant="primary" size="sm">
            save
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => setEditing(false)}
          >
            cancel
          </Button>
        </form>
      ) : (
        <p className="font-mono text-[13px] text-ink leading-snug">
          {memory.text}
        </p>
      )}
      <div className="flex items-center gap-2 flex-wrap">
        {score !== undefined ? (
          <span className="font-mono text-[10px] text-ink-ghost tabular-nums">
            {score.toFixed(2)}
          </span>
        ) : null}
        {memory.entities.map((entity) => (
          <Badge key={entity}>{entity}</Badge>
        ))}
        <span className="font-mono text-[10px] lowercase text-ink-ghost">
          {memory.confidence}
          {memory.corroboration > 0 && ` · seen ×${memory.corroboration + 1}`}
          {superseded && ' · superseded'}
        </span>
        <span className="flex-1" />
        <Button
          variant="icon"
          size="icon"
          onClick={() => onPin(memory)}
          disabled={busy || superseded}
          aria-label={memory.pinned ? 'unpin memory' : 'pin memory'}
          title={
            memory.pinned
              ? 'unpin (allows automatic consolidation again)'
              : 'pin (untouchable by every automatic path)'
          }
        >
          {memory.pinned ? (
            <PinOff className="w-3.5 h-3.5" aria-hidden />
          ) : (
            <Pin className="w-3.5 h-3.5" aria-hidden />
          )}
        </Button>
        <Button
          variant="icon"
          size="icon"
          onClick={() => {
            setEditText(memory.text)
            setEditing(true)
          }}
          disabled={busy || superseded}
          aria-label="edit memory"
        >
          <Pencil className="w-3.5 h-3.5" aria-hidden />
        </Button>
        <Button
          variant="icon"
          size="icon"
          onClick={() => onDelete(memory)}
          disabled={busy || superseded}
          aria-label="delete memory"
          title="tombstone (leaves recall; stays on disk under show history)"
        >
          <Trash2 className="w-3.5 h-3.5" aria-hidden />
        </Button>
      </div>
    </li>
  )
}

export function MemoriesPanel({
  bank,
  memories,
  total,
  offset,
  pageSize,
  onOffsetChange,
  includeSuperseded,
  onToggleSuperseded,
  onSave,
  onPin,
  onEdit,
  onDelete,
  busy,
}: MemoriesPanelProps) {
  const [draft, setDraft] = useState('')
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<
    { memory: MemoryItem; score: number }[] | null
  >(null)
  const [searching, setSearching] = useState(false)

  const searchMode = results !== null

  const runSearch = async () => {
    const q = query.trim()
    if (!q) {
      setResults(null)
      return
    }
    setSearching(true)
    try {
      const res = await recall(bank, q, 50)
      setResults(res.memories)
    } finally {
      setSearching(false)
    }
  }

  const page = Math.floor(offset / pageSize) + 1
  const pages = Math.max(1, Math.ceil(total / pageSize))

  return (
    <div className="flex flex-col gap-3">
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          const text = draft.trim()
          if (text.length < 3 || busy) return
          void onSave(text).then((ok) => {
            if (ok) setDraft('')
          })
        }}
      >
        <Input
          value={draft}
          onChange={setDraft}
          preserveCase
          placeholder="remember something..."
          aria-label="new memory"
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

      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          void runSearch()
        }}
      >
        <Input
          value={query}
          onChange={(next) => {
            setQuery(next)
            if (!next.trim()) setResults(null)
          }}
          preserveCase
          placeholder={`search ${total} memories (ranked recall)...`}
          aria-label="search memories"
          className="flex-1"
        />
        <Button
          type="submit"
          variant="ghost"
          size="sm"
          disabled={!query.trim() || searching}
          className="gap-1"
        >
          <Search className="w-3.5 h-3.5" aria-hidden />
          search
        </Button>
        {searchMode ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              setQuery('')
              setResults(null)
            }}
            className="gap-1"
          >
            <X className="w-3.5 h-3.5" aria-hidden />
            clear
          </Button>
        ) : null}
      </form>

      <div className="flex items-center justify-between gap-2 flex-wrap">
        <span className="font-mono text-[11px] lowercase text-ink-faint">
          {searchMode
            ? `${results.length} matches (top 50 by score)`
            : `${total} memories · page ${page}/${pages}`}
        </span>
        <div className="flex items-center gap-2">
          {!searchMode && pages > 1 ? (
            <span className="flex items-center gap-1">
              <Button
                variant="icon"
                size="icon"
                disabled={offset === 0}
                onClick={() => onOffsetChange(Math.max(0, offset - pageSize))}
                aria-label="previous page"
              >
                <ChevronLeft className="w-3.5 h-3.5" aria-hidden />
              </Button>
              <Button
                variant="icon"
                size="icon"
                disabled={offset + pageSize >= total}
                onClick={() => onOffsetChange(offset + pageSize)}
                aria-label="next page"
              >
                <ChevronRight className="w-3.5 h-3.5" aria-hidden />
              </Button>
            </span>
          ) : null}
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
      </div>

      {searchMode ? (
        results.length === 0 ? (
          <EmptyState
            title="nothing matched"
            description="recall matches on words and entity handles. try different terms or clear the search."
          />
        ) : (
          <ul className="border border-rule divide-y divide-rule-2">
            {results.map(({ memory, score }) => (
              <FactRow
                key={memory.id}
                memory={memory}
                score={score}
                onPin={onPin}
                onEdit={onEdit}
                onDelete={onDelete}
                busy={busy}
              />
            ))}
          </ul>
        )
      ) : memories.length === 0 ? (
        <EmptyState
          title="no memories in this bank yet"
          description="memories arrive automatically after each completed turn, or save one above. sessions pick this bank via session metadata memory_bank."
        />
      ) : (
        <ul className="border border-rule divide-y divide-rule-2">
          {memories.map((memory) => (
            <FactRow
              key={memory.id}
              memory={memory}
              onPin={onPin}
              onEdit={onEdit}
              onDelete={onDelete}
              busy={busy}
            />
          ))}
        </ul>
      )}
    </div>
  )
}
