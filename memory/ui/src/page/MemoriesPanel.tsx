import { useMemo, useState } from 'react'
import { Badge, Button, EmptyState, type Host, Input } from '@iii-dev/console-ui'
import {
  ChevronLeft,
  ChevronRight,
  MessageSquare,
  Pencil,
  Pin,
  PinOff,
  Search,
  Trash2,
  X,
} from './icons'
import { type MemoryItem, recall } from './memory-data'

/**
 * The selected bank's memories, built for large banks: server-side pages of
 * `pageSize` (newest first) with explicit paging controls, and a search
 * box that runs `memory::recall` (the ranked scorer, not a client filter)
 * so finding one memory among 10k costs one call. Every row is editable in
 * place: pin, edit text (revision bump), tombstone delete.
 */

/** "2h ago" style relative time; day precision past a week. */
export function timeAgo(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.floor((now - ms) / 1000))
  if (s < 60) return 'just now'
  if (s < 3_600) return `${Math.floor(s / 60)}m ago`
  if (s < 86_400) return `${Math.floor(s / 3_600)}h ago`
  if (s < 7 * 86_400) return `${Math.floor(s / 86_400)}d ago`
  return new Date(ms).toLocaleDateString()
}

/** Per-day capture counts for the last `days`, oldest first. */
export function activityBuckets(
  createdAts: number[],
  days: number,
  now = Date.now(),
): number[] {
  const buckets = new Array(days).fill(0)
  const dayMs = 86_400_000
  for (const at of createdAts) {
    const age = Math.floor((now - at) / dayMs)
    if (age >= 0 && age < days) buckets[days - 1 - age] += 1
  }
  return buckets
}

interface MemoriesPanelProps {
  host: Host
  bank: string
  memories: MemoryItem[]
  total: number
  offset: number
  pageSize: number
  onOffsetChange: (next: number) => void
  includeSuperseded: boolean
  onToggleSuperseded: (next: boolean) => void
  tag: string | null
  onTagChange: (next: string | null) => void
  tags: { tag: string; count: number }[]
  onOpenChat: (sessionId: string) => void
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
  onOpenChat,
  busy,
  score,
}: {
  memory: MemoryItem
  onPin: (memory: MemoryItem) => void
  onEdit: (memory: MemoryItem, text: string) => Promise<boolean>
  onDelete: (memory: MemoryItem) => void
  onOpenChat: (sessionId: string) => void
  busy: boolean
  score?: number
}) {
  const [editing, setEditing] = useState(false)
  const [editText, setEditText] = useState(memory.text)
  const superseded = memory.invalid_at != null

  return (
    <li
      className={`mem-fact${memory.pinned ? ' pinned' : ''}${superseded ? ' superseded' : ''}`}
    >
      {editing ? (
        <form
          className="mem-row"
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
            className="mem-flex1"
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
        <p className="mem-fact-text">{memory.text}</p>
      )}
      <div className="mem-fact-meta">
        {score !== undefined ? (
          <span className="mem-score">{score.toFixed(2)}</span>
        ) : null}
        {memory.entities.map((entity) => (
          <Badge key={entity}>{entity}</Badge>
        ))}
        {memory.tags.map((tag) => (
          <span key={tag} className="mem-tag">
            #{tag}
          </span>
        ))}
        <span className="mem-meta-note">
          {timeAgo(memory.created_at)}
          {memory.corroboration > 0 && ` · seen ×${memory.corroboration + 1}`}
          {memory.confidence === 'stated' && ' · saved explicitly'}
          {superseded && ' · superseded'}
        </span>
        {memory.source?.session_id ? (
          <button
            type="button"
            onClick={() => onOpenChat(memory.source?.session_id ?? '')}
            title="open the conversation this memory came from"
            className="mem-fromchat"
          >
            <MessageSquare size={12} aria-hidden />
            from chat
          </button>
        ) : null}
        <span className="mem-spacer" />
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
            <PinOff size={14} aria-hidden />
          ) : (
            <Pin size={14} aria-hidden />
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
          <Pencil size={14} aria-hidden />
        </Button>
        <Button
          variant="icon"
          size="icon"
          onClick={() => onDelete(memory)}
          disabled={busy || superseded}
          aria-label="delete memory"
          title="tombstone (leaves recall; stays on disk under show history)"
        >
          <Trash2 size={14} aria-hidden />
        </Button>
      </div>
    </li>
  )
}

export function MemoriesPanel({
  host,
  bank,
  memories,
  total,
  offset,
  pageSize,
  onOffsetChange,
  includeSuperseded,
  onToggleSuperseded,
  tag,
  onTagChange,
  tags,
  onOpenChat,
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
      const res = await recall(host, bank, q, 50)
      setResults(res.memories)
    } finally {
      setSearching(false)
    }
  }

  const page = Math.floor(offset / pageSize) + 1
  const pages = Math.max(1, Math.ceil(total / pageSize))

  const buckets = useMemo(
    () => activityBuckets(memories.map((m) => m.created_at), 30),
    [memories],
  )
  const capturedThisWeek = useMemo(
    () => buckets.slice(-7).reduce((a, b) => a + b, 0),
    [buckets],
  )
  const maxBucket = Math.max(1, ...buckets)

  return (
    <div className="mem-stack">
      <p className="mem-hint">
        one line per durable thing said in chat — captured automatically after
        each turn, each with the conversation it came from. a memory reaches
        the agent only when it matches the question being asked; rules are the
        always-on half.
      </p>

      {total > 0 ? (
        <div className="mem-activity">
          <div className="mem-bars" aria-hidden>
            {buckets.map((count, i) => (
              <span
                // biome-ignore lint/suspicious/noArrayIndexKey: fixed-size day series
                key={i}
                className={`mem-bar${count > 0 ? ' on' : ''}`}
                style={{
                  height: `${count > 0 ? Math.max(15, Math.round((count / maxBucket) * 100)) : 6}%`,
                }}
              />
            ))}
          </div>
          <div className="mem-activity-text">
            <span className="mem-activity-count">
              {capturedThisWeek > 0
                ? `${capturedThisWeek} captured this week`
                : 'nothing captured this week'}
            </span>
            <span className="mem-subhint">
              last 30 days · {total} total · chat and this page stay in sync
              live
            </span>
          </div>
        </div>
      ) : null}

      <form
        className="mem-row"
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
          className="mem-flex1"
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
        className="mem-row"
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
          className="mem-flex1"
        />
        <Button
          type="submit"
          variant="ghost"
          size="sm"
          disabled={!query.trim() || searching}
          className="mem-gap1"
        >
          <Search size={14} aria-hidden />
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
            className="mem-gap1"
          >
            <X size={14} aria-hidden />
            clear
          </Button>
        ) : null}
      </form>

      {tags.length > 0 ? (
        <div className="mem-row wrap">
          <span className="mem-caption">tags</span>
          {tags.map(({ tag: t, count }) => (
            <button
              key={t}
              type="button"
              onClick={() => onTagChange(tag === t ? null : t)}
              className={`mem-tagbtn${tag === t ? ' active' : ''}`}
            >
              #{t} · {count}
            </button>
          ))}
          {tag ? (
            <button
              type="button"
              onClick={() => onTagChange(null)}
              className="mem-linklike"
            >
              clear
            </button>
          ) : null}
        </div>
      ) : null}

      <div className="mem-spread wrap">
        <span className="mem-hint">
          {searchMode
            ? `${results.length} matches (top 50 by score)`
            : `${total} memories · page ${page}/${pages}`}
        </span>
        <div className="mem-row">
          {!searchMode && pages > 1 ? (
            <span className="mem-row tight">
              <Button
                variant="icon"
                size="icon"
                disabled={offset === 0}
                onClick={() => onOffsetChange(Math.max(0, offset - pageSize))}
                aria-label="previous page"
              >
                <ChevronLeft size={14} aria-hidden />
              </Button>
              <Button
                variant="icon"
                size="icon"
                disabled={offset + pageSize >= total}
                onClick={() => onOffsetChange(offset + pageSize)}
                aria-label="next page"
              >
                <ChevronRight size={14} aria-hidden />
              </Button>
            </span>
          ) : null}
          <label className="mem-check">
            <input
              type="checkbox"
              checked={includeSuperseded}
              onChange={(e) => onToggleSuperseded(e.target.checked)}
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
          <ul className="mem-fact-list">
            {results.map(({ memory, score }) => (
              <FactRow
                key={memory.id}
                memory={memory}
                score={score}
                onPin={onPin}
                onEdit={onEdit}
                onDelete={onDelete}
                onOpenChat={onOpenChat}
                busy={busy}
              />
            ))}
          </ul>
        )
      ) : memories.length === 0 ? (
        <EmptyState
          title="nothing remembered yet"
          description="pick this bank in the chat composer, then just talk — say something durable ('our api port is 3111') and it appears here after the turn, linked to that conversation. or type one above."
        />
      ) : (
        <ul className="mem-fact-list">
          {memories.map((memory) => (
            <FactRow
              key={memory.id}
              memory={memory}
              onPin={onPin}
              onEdit={onEdit}
              onDelete={onDelete}
              onOpenChat={onOpenChat}
              busy={busy}
            />
          ))}
        </ul>
      )}
    </div>
  )
}
