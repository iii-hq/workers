import { Search } from 'lucide-react'
import { useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import { type RecalledMemory, recall } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * Recall dry-run: the exact scorer the pre-generate hook uses, so a user
 * can preview precisely which memories a turn on some topic would be given
 * (and why — scores shown). Zero LLM; instant.
 */

interface RecallPanelProps {
  bank: string
}

export function RecallPanel({ bank }: RecallPanelProps) {
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<RecalledMemory[] | null>(null)
  const [retrieval, setRetrieval] = useState('')
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    const q = query.trim()
    if (!q) return
    setRunning(true)
    setError(null)
    try {
      const res = await recall(bank, q)
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
        preview exactly what a turn on this topic would be given — same scorer
        as the injection hook, no llm
      </p>
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          void run()
        }}
      >
        <Input
          value={query}
          onChange={setQuery}
          preserveCase
          placeholder="what would the agent recall about..."
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

      {error ? (
        <p className="font-mono text-[12px] lowercase text-alert">{error}</p>
      ) : null}

      {results !== null && !error ? (
        results.length === 0 ? (
          <EmptyState
            title="nothing recalled"
            description="no memories in this bank matched the query. memories match on words and entity handles."
          />
        ) : (
          <div className="flex flex-col gap-2">
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              retrieval: {retrieval || 'bm25-entity'}
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
