import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Input } from '@/components/ui/Input'
import {
  type ConversationPreview,
  type ConversationSource,
  conversationSources,
  type Discovery,
  discoverConversations,
  importConversation,
  previewConversation,
} from '@/lib/conversation-import'
import { errText } from '@/lib/errors'

/** What one "Import selected" run ended up doing, announced when it ends. */
interface ImportSummary {
  imported: number
  messages: number
  failed: number
  lastSessionId: string | null
}

export function summaryText(value: ImportSummary): string {
  const failures =
    value.failed === 0
      ? ''
      : value.failed === 1
        ? ' · 1 failed'
        : ` · ${value.failed} failed`
  if (value.imported === 0) return `Nothing imported${failures}`
  const conversations =
    value.imported === 1 ? '1 conversation' : `${value.imported} conversations`
  const messages =
    value.messages === 1 ? '1 message' : `${value.messages} messages`
  return `Imported ${conversations} · ${messages}${failures}`
}

export function ImportConversationsDialog({
  open,
  onOpenChange,
  onImported,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImported: (id: string) => void
}) {
  const [source, setSource] = useState<ConversationSource>('codex')
  const [query, setQuery] = useState('')
  const [search, setSearch] = useState('')
  const [retry, setRetry] = useState(0)
  const generation = useRef(0)
  const [discovery, setDiscovery] = useState<Discovery | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [previewId, setPreviewId] = useState<string | null>(null)
  const [preview, setPreview] = useState<ConversationPreview | null>(null)
  const [error, setError] = useState('')
  const [previewError, setPreviewError] = useState('')
  const [loading, setLoading] = useState(false)
  const [importing, setImporting] = useState(false)
  const [results, setResults] = useState<Record<string, string>>({})
  const [summary, setSummary] = useState<ImportSummary | null>(null)
  // biome-ignore lint/correctness/useExhaustiveDependencies: retry explicitly reruns the same discovery request.
  useEffect(() => {
    generation.current += 1
    if (!open) return
    let cancelled = false
    setLoading(true)
    setDiscovery(null)
    setSelected(new Set())
    setPreviewId(null)
    setResults({})
    setSummary(null)
    setError('')
    void discoverConversations({ source, query: search })
      .then((value) => {
        if (!cancelled) setDiscovery(value)
      })
      .catch((e) => {
        if (!cancelled) setError(errText(e))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [open, source, search, retry])
  useEffect(() => {
    setPreview(null)
    setPreviewError('')
    if (!previewId || !open) return
    let cancelled = false
    void previewConversation(source, previewId)
      .then((value) => {
        if (!cancelled) setPreview(value)
      })
      .catch((e) => {
        if (!cancelled) setPreviewError(errText(e))
      })
    return () => {
      cancelled = true
    }
  }, [source, previewId, open])
  async function loadMore() {
    if (!discovery?.next_cursor) return
    const requestGeneration = generation.current
    setLoading(true)
    setError('')
    try {
      const page = await discoverConversations({
        source,
        query: search,
        cursor: discovery.next_cursor,
      })
      if (generation.current !== requestGeneration) return
      setDiscovery({
        ...page,
        conversations: [...discovery.conversations, ...page.conversations],
      })
    } catch (e) {
      if (generation.current === requestGeneration) setError(errText(e))
    } finally {
      if (generation.current === requestGeneration) setLoading(false)
    }
  }
  async function importSelected() {
    setImporting(true)
    setSummary(null)
    let imported = 0
    let messages = 0
    let failed = 0
    let lastSessionId: string | null = null
    for (const id of selected) {
      setResults((value) => ({ ...value, [id]: 'Importing…' }))
      try {
        const result = await importConversation(source, id)
        imported += 1
        messages += result.total_messages
        lastSessionId = result.session_id
        setResults((value) => ({
          ...value,
          [id]: `Imported · ${result.total_messages} messages`,
        }))
      } catch (e) {
        failed += 1
        setResults((value) => ({ ...value, [id]: `Failed: ${errText(e)}` }))
      }
    }
    // The batch is over: say so once, for the whole selection. The per-row
    // lines can be scrolled out of view, and nothing else in the console
    // announces a conversation that was created behind this dialog.
    setSummary({ imported, messages, failed, lastSessionId })
    setImporting(false)
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!importing) onOpenChange(next)
      }}
    >
      <DialogContent className="flex max-h-[90dvh] w-[calc(100%-2rem)] max-w-4xl flex-col gap-4 overflow-hidden">
        <DialogTitle>Import conversations</DialogTitle>
        <DialogDescription>
          Read user and assistant text from histories on the machine running
          ADE. Each import creates a new conversation you can continue in ADE.
          Tools, attachments and subagents are excluded.
        </DialogDescription>
        <div className="flex flex-wrap gap-2">
          {(Object.keys(conversationSources) as ConversationSource[]).map(
            (value) => (
              <Button
                key={value}
                variant={source === value ? 'primary' : 'ghost'}
                disabled={importing || loading}
                aria-pressed={source === value}
                onClick={() => setSource(value)}
              >
                {conversationSources[value]}
              </Button>
            ),
          )}
        </div>
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            setSearch(query.trim())
          }}
        >
          <Input
            name="import-search"
            aria-label="Search history titles and projects"
            placeholder="Search titles and projects"
            value={query}
            onChange={setQuery}
            disabled={importing || loading}
          />
          <Button type="submit" disabled={importing || loading}>
            Search
          </Button>
        </form>
        {discovery && (
          <p className="break-all text-xs text-ink-faint">
            Directory on the ADE host: {discovery.directory}
          </p>
        )}
        {discovery?.warnings?.map((warning) => (
          <p key={warning} className="text-xs text-ink-faint">
            {warning}
          </p>
        ))}
        {error && (
          <div role="alert" className="text-sm text-alert">
            <p>{error}</p>
            <Button
              disabled={loading || importing}
              onClick={() => setRetry((value) => value + 1)}
            >
              Retry discovery
            </Button>
          </div>
        )}
        {loading && (
          <p role="status" className="text-sm text-ink-faint">
            Finding conversations…
          </p>
        )}
        {discovery && !discovery.available && (
          <p className="text-sm text-ink-faint">
            No accessible {conversationSources[source]} history found. When ADE
            runs in a container, the history directory must be mounted there.
          </p>
        )}
        {discovery?.available && discovery.conversations.length === 0 && (
          <p className="text-sm text-ink-faint">
            {search
              ? 'No conversations match this search.'
              : 'No conversations found.'}
          </p>
        )}
        <div className="grid min-h-0 flex-1 gap-4 overflow-y-auto sm:grid-cols-2">
          <section
            className="min-w-0 space-y-2 sm:overflow-y-auto"
            aria-label="Available histories"
          >
            {discovery?.conversations.map((item) => (
              <div key={item.id} className="rounded-md bg-surface p-3">
                <label className="flex items-start gap-2 text-sm">
                  <input
                    type="checkbox"
                    className="mt-1"
                    checked={selected.has(item.id)}
                    disabled={importing}
                    onChange={(e) => {
                      setSelected((value) => {
                        const next = new Set(value)
                        if (e.target.checked) next.add(item.id)
                        else next.delete(item.id)
                        return next
                      })
                    }}
                  />
                  <span className="min-w-0 break-words">
                    {item.title || item.id}
                  </span>
                </label>
                <p className="mt-1 break-all text-xs text-ink-faint">
                  {item.cwd ?? 'No project'} ·{' '}
                  {new Date(item.updated_at).toLocaleString()}
                </p>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={importing}
                  onClick={() => setPreviewId(item.id)}
                >
                  Preview
                </Button>
                {results[item.id] && (
                  <p
                    role="status"
                    className="break-words text-xs text-ink-faint"
                  >
                    {results[item.id]}
                  </p>
                )}
              </div>
            ))}
            {discovery?.next_cursor && (
              <Button
                disabled={loading || importing}
                onClick={() => void loadMore()}
              >
                Load more
              </Button>
            )}
          </section>
          <section
            aria-label="History preview"
            className="min-w-0 space-y-3 sm:overflow-y-auto"
          >
            {!previewId && (
              <p className="text-sm text-ink-faint">
                Preview a conversation before importing.
              </p>
            )}
            {previewId && !preview && !previewError && (
              <p role="status">Loading preview…</p>
            )}
            {previewError && <p role="alert">{previewError}</p>}
            {preview?.warnings.map((warning) => (
              <p key={warning} className="text-xs text-ink-faint">
                {warning}
              </p>
            ))}
            {preview?.messages.length === 0 && (
              <p>No user or assistant text to import.</p>
            )}
            {preview?.messages.map((message) => (
              <article
                key={message.id}
                className="rounded-md bg-surface p-3 text-sm"
              >
                <h3 className="mb-1 font-medium">
                  {message.role === 'user'
                    ? 'User'
                    : conversationSources[source]}
                </h3>
                <p className="whitespace-pre-wrap break-words">
                  {message.text}
                </p>
              </article>
            ))}
          </section>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          {summary && (
            <p
              role={summary.imported > 0 ? 'status' : 'alert'}
              className={`mr-auto break-words text-sm ${
                summary.imported > 0 ? 'text-ink-faint' : 'text-alert'
              }`}
            >
              {summaryText(summary)}
            </p>
          )}
          <Button disabled={importing} onClick={() => onOpenChange(false)}>
            Close
          </Button>
          {summary?.lastSessionId && (
            <Button
              variant="primary"
              disabled={importing}
              onClick={() => {
                const id = summary.lastSessionId
                if (id) onImported(id)
                onOpenChange(false)
              }}
            >
              {summary.imported > 1
                ? 'Open last imported'
                : 'Open conversation'}
            </Button>
          )}
          <Button
            variant={summary ? 'ghost' : 'primary'}
            disabled={selected.size === 0 || importing || loading}
            onClick={() => void importSelected()}
          >
            {importing ? 'Importing…' : `Import selected (${selected.size})`}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
