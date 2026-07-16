import { Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import { type MemoryItem, type RecalledMemory, recall } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * Recall dry-run: the exact scorer the pre-generate hook uses, so a user
 * can preview precisely which memories a turn on some topic would be given
 * (and why — scores shown). Zero LLM; instant. First-use DX: the panel
 * explains itself in chat terms and offers clickable example questions
 * derived from THIS bank's content, so the first recall is one click.
 */

interface RecallPanelProps {
  bank: string
  memories: MemoryItem[]
  tags: { tag: string; count: number }[]
}

/** Example questions from the bank's own content: its busiest tags, its
 * entities, and the identity question every session opener implies. */
function suggestions(
  memories: MemoryItem[],
  tags: { tag: string; count: number }[],
): string[] {
  const out: string[] = []
  for (const { tag } of tags.slice(0, 2)) {
    out.push(`what do we know about ${tag}?`)
  }
  const entities = new Set<string>()
  for (const m of memories) {
    for (const e of m.entities) {
      if (e !== 'user') entities.add(e)
    }
  }
  for (const entity of [...entities].slice(0, 2)) {
    out.push(`tell me about ${entity}`)
  }
  out.push('who is the user?')
  return [...new Set(out)].slice(0, 4)
}

export function RecallPanel({ bank, memories, tags }: RecallPanelProps) {
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<RecalledMemory[] | null>(null)
  const [retrieval, setRetrieval] = useState('')
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const examples = useMemo(() => suggestions(memories, tags), [memories, tags])

  const run = async (q: string) => {
    const trimmed = q.trim()
    if (!trimmed) return
    setRunning(true)
    setError(null)
    try {
      const res = await recall(bank, trimmed)
      setResults(res.memories)
      setRetrieval(res.retrieval)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setResults(null)
    } finally {
      setRunning(false)
    }
  }

  const maxScore = results?.[0]?.score ?? 1

  return (
    <div className="flex flex-col gap-3">
      <p className="font-mono text-[11px] lowercase text-ink-faint">
        a dry-run of the agent's memory: type what someone might ask in chat,
        and see the exact memories that turn would be handed — same scorer the
        injection hook runs, scored and ranked, no model involved.
      </p>
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          void run(query)
        }}
      >
        <Input
          value={query}
          onChange={setQuery}
          preserveCase
          placeholder="ask like a chat user would — e.g. when do I publish?"
          aria-label="recall query"
          className="flex-1"
        />
        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={!query.trim() || running}
          className="gap-1"
        >
          <Search className="w-3.5 h-3.5" aria-hidden />
          recall
        </Button>
      </form>

      {results === null && !error ? (
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-ghost">
            try
          </span>
          {examples.map((example) => (
            <button
              key={example}
              type="button"
              onClick={() => {
                setQuery(example)
                void run(example)
              }}
              className="font-mono text-[11px] lowercase px-1.5 py-0.5 border border-rule text-ink-faint hover:border-ink hover:text-ink transition-colors"
            >
              {example}
            </button>
          ))}
        </div>
      ) : null}

      {error ? (
        <p className="font-mono text-[12px] lowercase text-alert">{error}</p>
      ) : null}

      {results !== null && !error ? (
        results.length === 0 ? (
          <EmptyState
            title="nothing recalled"
            description="no memories in this bank matched the query. memories match on words, entity handles, and meaning (when embeddings are configured) — try one of the suggestions, or phrase it with words the memories use."
          />
        ) : (
          <div className="flex flex-col gap-2">
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              what the turn would be given · retrieval:{' '}
              {retrieval || 'bm25-entity'}
            </span>
            <ul className="border border-rule divide-y divide-rule-2">
              {results.map(({ memory, score }) => (
                <li key={memory.id} className="px-3 py-2 flex flex-col gap-1">
                  <div className="flex items-center gap-2">
                    <span
                      className={cn('h-1.5 bg-accent shrink-0')}
                      style={{
                        width: `${Math.max(4, Math.round((score / maxScore) * 64))}px`,
                      }}
                      aria-hidden
                    />
                    <span className="font-mono text-[10px] text-ink-ghost tabular-nums">
                      {score.toFixed(2)}
                    </span>
                    {memory.pinned ? (
                      <Badge variant="accent">pinned</Badge>
                    ) : null}
                  </div>
                  <p className="font-mono text-[13px] text-ink leading-snug">
                    {memory.text}
                  </p>
                </li>
              ))}
            </ul>
          </div>
        )
      ) : null}
    </div>
  )
}
