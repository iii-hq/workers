import { useMemo, useState } from 'react'
import { Badge, Button, EmptyState, type Host, Input } from '@iii-dev/console-ui'
import { ChevronDown, ChevronRight, Search } from './icons'
import { type MemoryItem, preview, type TurnPreview } from './memory-data'

/**
 * Turn preview: not another search box (the memories tab has one) — this
 * composes the ENTIRE memory payload a chat turn on this bank would get,
 * via `memory::preview`, which runs the same code as the pre-generate
 * hook: the system-prompt section with rules and budgets applied, the
 * memories after the ambient floor and token budget, and the appended
 * message verbatim.
 */

interface RecallPanelProps {
  host: Host
  bank: string
  memories: MemoryItem[]
  tags: { tag: string; count: number }[]
}

const ghost = { color: 'var(--color-ink-ghost)' } as const

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

export function RecallPanel({ host, bank, memories, tags }: RecallPanelProps) {
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
      setResult(await preview(host, bank, trimmed))
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setResult(null)
    } finally {
      setRunning(false)
    }
  }

  const maxScore = result?.memories[0]?.score || 1

  return (
    <div className="mem-ui-stack">
      <p className="mem-ui-hint">
        the whole turn, before it happens: type what someone would ask in chat
        and see everything memory hands that turn — the rules going into the
        system prompt (budgets and truncation applied) and the exact memories
        appended, in order. same code the live hook runs.
      </p>
      <form
        className="mem-ui-row"
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
          className="mem-ui-flex1"
        />
        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={!query.trim() || running}
          className="mem-ui-gap1"
        >
          <Search size={14} aria-hidden />
          preview turn
        </Button>
      </form>

      {result === null && !error ? (
        <div className="mem-ui-row wrap">
          <span className="mem-ui-caption">try</span>
          {examples.map((example) => (
            <button
              key={example}
              type="button"
              onClick={() => {
                setQuery(example)
                void run(example)
              }}
              className="mem-ui-tagbtn"
            >
              {example}
            </button>
          ))}
        </div>
      ) : null}

      {error ? <p className="mem-ui-error-text">{error}</p> : null}

      {result !== null && !error ? (
        <div className="mem-ui-stack">
          <div className="mem-ui-prompt">
            <button
              type="button"
              onClick={() => setShowPrompt((v) => !v)}
              className="mem-ui-prompt-head"
            >
              {showPrompt ? (
                <ChevronDown size={14} style={ghost} aria-hidden />
              ) : (
                <ChevronRight size={14} style={ghost} aria-hidden />
              )}
              <span className="mem-ui-prompt-label">
                system prompt gets: {result.rules} rule
                {result.rules === 1 ? '' : 's'}
                {result.rulesTruncated ? ' · over budget, truncated' : ''}
              </span>
              <span className="mem-ui-spacer" />
              <span className="mem-ui-subhint">every turn, guaranteed</span>
            </button>
            {showPrompt ? (
              <pre className="mem-ui-prompt-body">
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
            <div className="mem-ui-stack tight">
              <span className="mem-ui-subhint">
                appended to the turn ({result.memories.length}, in order) ·
                retrieval: {result.retrieval || 'bm25-entity'}
              </span>
              <ul className="mem-ui-fact-list">
                {result.memories.map(({ memory, score }) => (
                  <li key={memory.id} className="mem-ui-recall-item">
                    <div className="mem-ui-row">
                      <span
                        className={`mem-ui-scorebar${score > 0 ? ' on' : ''}`}
                        style={{
                          width: `${Math.max(4, Math.round((score / maxScore) * 64))}px`,
                        }}
                        aria-hidden
                      />
                      <span className="mem-ui-score">
                        {score > 0 ? score.toFixed(2) : 'ambient'}
                      </span>
                      {memory.pinned ? (
                        <Badge variant="accent">pinned</Badge>
                      ) : null}
                    </div>
                    <p className="mem-ui-fact-text">{memory.text}</p>
                  </li>
                ))}
              </ul>
              <p className="mem-ui-subhint">
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
