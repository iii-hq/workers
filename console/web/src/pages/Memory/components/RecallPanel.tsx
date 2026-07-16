import { ChevronDown, ChevronRight, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import { type MemoryItem, type TurnPreview, preview } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * Turn preview: not another search box (the memories tab has one) — this
 * composes the ENTIRE memory payload a chat turn on this bank would get,
 * via `memory::preview`, which runs the same code as the pre-generate
 * hook: the system-prompt section with rules and budgets applied, the
 * memories after the ambient floor and token budget, and the appended
 * message verbatim.
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
  const [result, setResult] = useState<TurnPreview | null>(null)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [showPrompt, setShowPrompt] = useState(false)

  const examples = useMemo(() => suggestions(memories, tags), [memories, tags])

  const run = async (q: string) => {
    const trimmed = q.trim()
    if (!trimmed) return
    setRunning(true)
    setError(null)
    try {
      setResult(await preview(bank, trimmed))
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setResult(null)
    } finally {
      setRunning(false)
    }
  }

  const maxScore = result?.memories[0]?.score || 1

  return (
    <div className="flex flex-col gap-3">
      <p className="font-mono text-[11px] lowercase text-ink-faint">
        the whole turn, before it happens: type what someone would ask in chat
        and see everything memory hands that turn — the rules going into the
        system prompt (budgets and truncation applied) and the exact memories
        appended, in order. same code the live hook runs.
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
          aria-label="turn preview query"
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
          preview turn
        </Button>
      </form>

      {result === null && !error ? (
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

      {result !== null && !error ? (
        <div className="flex flex-col gap-3">
          <div className="border border-rule">
            <button
              type="button"
              onClick={() => setShowPrompt((v) => !v)}
              className="w-full flex items-center gap-2 px-3 py-2 text-left"
            >
              {showPrompt ? (
                <ChevronDown className="w-3.5 h-3.5 text-ink-ghost" aria-hidden />
              ) : (
                <ChevronRight className="w-3.5 h-3.5 text-ink-ghost" aria-hidden />
              )}
              <span className="font-mono text-[11px] lowercase text-ink">
                system prompt gets: {result.rules} rule
                {result.rules === 1 ? '' : 's'}
                {result.rulesTruncated ? ' · over budget, truncated' : ''}
              </span>
              <span className="flex-1" />
              <span className="font-mono text-[10px] lowercase text-ink-ghost">
                every turn, guaranteed
              </span>
            </button>
            {showPrompt ? (
              <pre className="px-3 pb-3 font-mono text-[11px] text-ink-faint whitespace-pre-wrap leading-relaxed border-t border-rule-2 pt-2 max-h-72 overflow-auto">
                {result.systemPromptSection.trim()}
              </pre>
            ) : null}
          </div>

          {result.memories.length === 0 ? (
            <EmptyState
              title="no memories would be appended"
              description="nothing in this bank matches this question and nothing is strong enough for the ambient floor. the rules above still land."
            />
          ) : (
            <div className="flex flex-col gap-2">
              <span className="font-mono text-[10px] lowercase text-ink-ghost">
                appended to the turn ({result.memories.length}, in order) ·
                retrieval: {result.retrieval || 'bm25-entity'}
              </span>
              <ul className="border border-rule divide-y divide-rule-2">
                {result.memories.map(({ memory, score }) => (
                  <li key={memory.id} className="px-3 py-2 flex flex-col gap-1">
                    <div className="flex items-center gap-2">
                      <span
                        className={cn('h-1.5 shrink-0', score > 0 ? 'bg-accent' : 'bg-rule')}
                        style={{
                          width: `${Math.max(4, Math.round((score / maxScore) * 64))}px`,
                        }}
                        aria-hidden
                      />
                      <span className="font-mono text-[10px] text-ink-ghost tabular-nums">
                        {score > 0 ? score.toFixed(2) : 'ambient'}
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
              <p className="font-mono text-[10px] lowercase text-ink-ghost">
                "ambient" = didn't match the question, but strong enough that
                every turn gets it (pinned and most-seen memories)
              </p>
            </div>
          )}
        </div>
      ) : null}
    </div>
  )
}
