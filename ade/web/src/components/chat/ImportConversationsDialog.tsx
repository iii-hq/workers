import {
  Check,
  CircleCheck,
  Copy,
  Folder,
  FolderX,
  Info,
  Loader2,
  RotateCw,
  TriangleAlert,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Badge, type BadgeVariant } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { Checkbox } from '@/components/ui/Checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { EmptyState } from '@/components/ui/EmptyState'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import { SearchField } from '@/components/ui/SearchField'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { copyTextToClipboard } from '@/lib/clipboard'
import {
  type ConversationPreview,
  type ConversationSource,
  conversationSources,
  type Discovery,
  discoverConversations,
  type ExternalConversation,
  importConversation,
  type PreviewMessage,
  previewConversation,
} from '@/lib/conversation-import'
import { errText } from '@/lib/errors'
import { formatElapsed } from '@/lib/relative-time'
import { cn } from '@/lib/utils'

const SOURCE_OPTIONS = (
  Object.keys(conversationSources) as ConversationSource[]
).map((value) => ({ value, label: conversationSources[value] }))
const SEARCH_DEBOUNCE_MS = 250

/** What one import run ended up doing, announced when it ends. */
export interface ImportSummary {
  imported: number
  messages: number
  commands: number
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
  const commands =
    value.commands === 0
      ? ''
      : value.commands === 1
        ? ' · 1 command'
        : ` · ${value.commands} commands`
  return `Imported ${conversations} · ${messages}${commands}${failures}`
}

/** The last segment of a source project path, or a placeholder when the history has none. */
export function projectLabel(cwd: string | null | undefined): string {
  return (
    cwd
      ?.split(/[\\/]+/)
      .filter(Boolean)
      .pop() ?? 'No project'
  )
}

/** "2h ago" for a source timestamp, in the sidebar's own elapsed copy. */
export function whenLabel(timestamp: number, now = Date.now()): string {
  const elapsed = formatElapsed(timestamp, now)
  if (elapsed === null) return ''
  return elapsed === 'just now' ? elapsed : `${elapsed} ago`
}

function clip(value: string, limit: number): string {
  return value.length > limit ? `${value.slice(0, limit - 1)}…` : value
}

/** `key: "value…", key2: [3]` for a call's first arguments, the way the chat's function rows digest them. */
export function argumentsDigest(input: unknown, limit = 48): string {
  if (input === null || input === undefined) return ''
  if (typeof input !== 'object') return clip(JSON.stringify(input), limit)
  return Object.entries(input as Record<string, unknown>)
    .slice(0, 2)
    .map(([key, value]) => {
      const rendered =
        typeof value === 'string'
          ? JSON.stringify(value)
          : Array.isArray(value)
            ? `[${value.length}]`
            : value !== null && typeof value === 'object'
              ? '{…}'
              : String(value)
      return `${key}: ${clip(rendered, limit)}`
    })
    .join(', ')
}

export type PreviewRow =
  | { kind: 'text'; id: string; role: 'user' | 'assistant'; text: string }
  | {
      kind: 'call'
      id: string
      functionId: string
      digest: string
      failed: boolean
      answered: boolean
    }

/**
 * The preview in reading order: text turns, and one function row per call
 * placed where the call happened, carrying the recorded result's outcome.
 * A result whose call is missing from the window still shows as a row.
 */
export function previewRows(messages: PreviewMessage[]): PreviewRow[] {
  const results = new Map<string, PreviewMessage>()
  for (const message of messages) {
    if (message.role === 'function_result' && message.call_id) {
      results.set(message.call_id, message)
    }
  }
  const claimed = new Set<string>()
  const rows: PreviewRow[] = []
  for (const message of messages) {
    if (message.role === 'function_result') {
      if (message.call_id && claimed.has(message.call_id)) continue
      rows.push({
        kind: 'call',
        id: message.id,
        functionId: message.function_id ?? 'tool',
        digest: '',
        failed: message.is_error === true,
        answered: true,
      })
      continue
    }
    if (message.text.trim()) {
      rows.push({
        kind: 'text',
        id: message.id,
        role: message.role,
        text: message.text,
      })
    }
    for (const call of message.calls ?? []) {
      claimed.add(call.id)
      const result = results.get(call.id)
      rows.push({
        kind: 'call',
        id: `${message.id}:${call.id}`,
        functionId: call.function_id,
        digest: argumentsDigest(call.arguments),
        failed: result?.is_error === true,
        answered: result !== undefined,
      })
    }
  }
  return rows
}

export function previewCounts(messages: PreviewMessage[]): {
  messages: number
  commands: number
} {
  let text = 0
  let commands = 0
  for (const message of messages) {
    if (message.role === 'function_result') commands += 1
    else if (message.text.trim()) text += 1
  }
  return { messages: text, commands }
}

function countsLabel(counts: { messages: number; commands: number }): string {
  const messages =
    counts.messages === 1 ? '1 message' : `${counts.messages} messages`
  if (counts.commands === 0) return messages
  const commands =
    counts.commands === 1 ? '1 command' : `${counts.commands} commands`
  return `${messages} · ${commands}`
}

type RowStatus = 'queued' | 'importing' | 'done' | 'failed'
const ROW_STATUS: Record<RowStatus, { label: string; variant: BadgeVariant }> =
  {
    queued: { label: 'Queued', variant: 'default' },
    importing: { label: 'Importing', variant: 'accent' },
    done: { label: 'Imported', variant: 'ok' },
    failed: { label: 'Failed', variant: 'alert' },
  }

export function HistoryRow({
  conversation,
  selected,
  open,
  status,
  failure,
  busy,
  now,
  onToggle,
  onOpen,
}: {
  conversation: ExternalConversation
  selected: boolean
  open: boolean
  status?: RowStatus
  failure?: string
  busy: boolean
  now?: number
  onToggle: () => void
  onOpen: () => void
}) {
  const title = conversation.title || conversation.id
  return (
    <div
      data-import-row={conversation.id}
      data-selected={selected || undefined}
      data-status={status}
      className={cn(
        'iii-ui-motion-control flex items-start gap-2.5 rounded-sm bg-surface px-3 py-2.5 transition-[background-color] hover:bg-surface-hover',
        (selected || open) && 'bg-surface-selected',
        status === 'done' && 'bg-ok-muted hover:bg-ok-muted',
        status === 'failed' && 'bg-alert-muted hover:bg-alert-muted',
      )}
    >
      <Checkbox
        className="mt-px shrink-0"
        checked={selected}
        disabled={busy}
        aria-label={`Select ${title}`}
        onChange={onToggle}
      />
      <button
        type="button"
        onClick={onOpen}
        aria-current={open || undefined}
        className="min-w-0 flex-1 rounded-xs text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
      >
        <span className="block truncate font-sans text-[13px] leading-[1.35] text-ink">
          {title}
        </span>
        <span className="mt-0.5 flex items-center gap-1.5 font-mono text-[11px] text-ink-ghost">
          <span className="truncate">{projectLabel(conversation.cwd)}</span>
          <span aria-hidden>·</span>
          <span className="shrink-0 tabular-nums">
            {whenLabel(conversation.updated_at, now)}
          </span>
        </span>
        {failure ? (
          <span
            className="mt-1 block truncate font-mono text-[11px] text-alert-strong"
            title={failure}
          >
            {failure}
          </span>
        ) : null}
      </button>
      {status ? (
        <Badge
          variant={ROW_STATUS[status].variant}
          className="mt-px shrink-0 gap-1"
        >
          {status === 'importing' ? <StatusDot tone="accent" pulse /> : null}
          {ROW_STATUS[status].label}
        </Badge>
      ) : null}
    </div>
  )
}

function SkeletonRow({ faded }: { faded?: number }) {
  return (
    <div
      className="flex items-start gap-2.5 rounded-sm bg-surface px-3 py-2.5"
      style={faded ? { opacity: faded } : undefined}
    >
      <Skeleton className="size-[18px]" />
      <div className="flex-1">
        <Skeleton className="block h-[9px] w-3/4" />
        <Skeleton className="mt-[7px] block h-2 w-1/2 opacity-60" />
      </div>
    </div>
  )
}

function PreviewSkeleton() {
  return (
    <div className="flex flex-col gap-2.5" aria-hidden>
      <div className="rounded-sm bg-surface px-3 py-2.5">
        <Skeleton className="block h-2 w-8" />
        <Skeleton className="mt-2 block h-[9px] w-full" />
        <Skeleton className="mt-1.5 block h-[9px] w-2/3" />
      </div>
      <div className="px-3 py-2.5">
        <Skeleton className="block h-2 w-10" />
        <Skeleton className="mt-2 block h-[9px] w-full" />
        <Skeleton className="mt-1.5 block h-[9px] w-5/6" />
      </div>
    </div>
  )
}

function TextRow({
  role,
  text,
  sourceLabel,
}: {
  role: 'user' | 'assistant'
  text: string
  sourceLabel: string
}) {
  return (
    <article
      data-preview-role={role}
      className={cn(
        'mb-2.5 rounded-sm px-3 py-2.5',
        role === 'user' && 'bg-surface',
      )}
    >
      <div className="mb-1 font-sans text-[11px] font-medium text-ink-faint">
        {role === 'user' ? 'You' : sourceLabel}
      </div>
      <p className="whitespace-pre-wrap break-words font-sans text-[13px] leading-[1.7] text-ink">
        {text}
      </p>
    </article>
  )
}

/** The chat's collapsed function row, read-only: outcome glyph, ƒ, verb, id, args digest. */
function CallRow({
  functionId,
  digest,
  failed,
  answered,
}: {
  functionId: string
  digest: string
  failed: boolean
  answered: boolean
}) {
  return (
    <div
      data-preview-role="function-call"
      data-function-id={functionId}
      data-function-status={failed ? 'error' : 'done'}
      className="mb-2.5 flex items-center gap-2 px-0.5 py-1 font-sans text-[13px] text-ink-faint"
    >
      {failed ? (
        <X
          className="size-4 shrink-0 stroke-alert"
          strokeWidth={2.5}
          aria-hidden
        />
      ) : (
        <Check
          className="size-4 shrink-0 stroke-ink-faint"
          strokeWidth={2.5}
          aria-hidden
        />
      )}
      <span
        aria-hidden
        className="flex size-4 shrink-0 items-center justify-center font-mono text-sm font-semibold italic text-accent"
      >
        ƒ
      </span>
      <span className="min-w-0 flex-1 truncate">
        {answered ? (failed ? 'Failed ' : 'Triggered ') : null}
        <span className="text-ink">{functionId}</span>
        {digest ? <span className="text-ink-ghost"> ({digest})</span> : null}
      </span>
    </div>
  )
}

export function PreviewPane({
  source,
  conversation,
  preview,
  error,
  loading,
}: {
  source: ConversationSource
  conversation: ExternalConversation | null
  preview: ConversationPreview | null
  error: string
  loading: boolean
}) {
  const sourceLabel = conversationSources[source]
  const rows = useMemo(
    () => (preview ? previewRows(preview.messages) : []),
    [preview],
  )
  const counts = preview ? previewCounts(preview.messages) : null
  return (
    <section
      aria-label="Preview"
      className="flex min-h-0 flex-col overflow-hidden rounded-md bg-surface"
    >
      <div className="shrink-0 bg-panel-raised px-3.5 py-2.5">
        {conversation ? (
          <>
            <div className="flex items-baseline gap-2.5">
              <div className="min-w-0 flex-1 truncate font-sans text-[13px] font-semibold text-ink">
                {conversation.title || conversation.id}
              </div>
              <div className="shrink-0 font-sans text-[11px] text-ink-faint">
                {sourceLabel}
              </div>
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-x-1.5 font-mono text-[11px] text-ink-ghost">
              <span className="max-w-full truncate">
                {conversation.cwd ?? 'No project'}
              </span>
              <span aria-hidden>·</span>
              <span className="shrink-0 tabular-nums">
                {new Date(conversation.updated_at).toLocaleString()}
              </span>
              {counts ? (
                <>
                  <span aria-hidden>·</span>
                  <span className="shrink-0 tabular-nums text-ink-faint">
                    {countsLabel(counts)}
                  </span>
                </>
              ) : null}
            </div>
          </>
        ) : (
          <div className="font-sans text-[13px] font-semibold text-ink-faint">
            Preview
          </div>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3.5">
        {!conversation ? (
          <EmptyState
            compact
            title="Nothing to preview"
            description="Select a conversation to preview it."
          />
        ) : error ? (
          <StatusPanel
            variant="alert"
            icon={<TriangleAlert className="h-full w-full" />}
            headline="Could not read this conversation"
            detail={<span className="font-mono break-all">{error}</span>}
          />
        ) : loading || !preview ? (
          <PreviewSkeleton />
        ) : (
          <>
            {preview.warnings.map((warning) => (
              <StatusPanel
                key={warning}
                variant="warn"
                className="mb-3"
                icon={<TriangleAlert className="h-full w-full" />}
                headline={warning}
              />
            ))}
            {rows.length === 0 ? (
              <EmptyState
                compact
                title="Nothing to import"
                description="This conversation has no messages or commands to bring across."
              />
            ) : null}
            {rows.map((row) =>
              row.kind === 'text' ? (
                <TextRow
                  key={row.id}
                  role={row.role}
                  text={row.text}
                  sourceLabel={sourceLabel}
                />
              ) : (
                <CallRow
                  key={row.id}
                  functionId={row.functionId}
                  digest={row.digest}
                  failed={row.failed}
                  answered={row.answered}
                />
              ),
            )}
          </>
        )}
      </div>
    </section>
  )
}

export function SourceUnavailable({
  source,
  onSwitch,
  onRetry,
}: {
  source: ConversationSource
  onSwitch: (next: ConversationSource) => void
  onRetry: () => void
}) {
  const other = SOURCE_OPTIONS.find((option) => option.value !== source)
  return (
    <EmptyState
      compact
      icon={FolderX}
      title={`No ${conversationSources[source]} history on this machine`}
      description="ADE looked in the directory above and found nothing readable. When ADE runs in a container, that directory has to be mounted into it."
      actions={[
        ...(other
          ? [
              {
                label: `Try ${other.label}`,
                onClick: () => onSwitch(other.value),
              },
            ]
          : []),
        { label: 'Check again', onClick: onRetry },
      ]}
    />
  )
}

export function DiscoveryFailed({
  error,
  onRetry,
}: {
  error: string
  onRetry: () => void
}) {
  const [copied, setCopied] = useState(false)
  return (
    <StatusPanel
      role="alert"
      variant="alert"
      icon={<TriangleAlert className="h-full w-full" />}
      headline="Could not read the history directory"
      detail={
        <>
          The import never started, so nothing was changed on disk.
          <code className="mt-2 block break-all rounded-sm bg-surface px-3 py-2 font-mono text-[11px] leading-[1.55]">
            {error}
          </code>
        </>
      }
      action={
        <div className="flex items-center gap-2">
          <Button variant="pill" size="sm" onClick={onRetry}>
            <RotateCw aria-hidden />
            Retry
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              void copyTextToClipboard(error).then((ok) => {
                if (!ok) return
                setCopied(true)
                window.setTimeout(() => setCopied(false), 1200)
              })
            }}
          >
            {copied ? <Check aria-hidden /> : <Copy aria-hidden />}
            {copied ? 'Copied' : 'Copy error'}
          </Button>
        </div>
      }
    />
  )
}

export function ImportFooter({
  phase,
  selectedCount,
  canImport,
  progress,
  summary,
  onCancel,
  onImport,
  onImportMore,
  onRetryFailed,
  onOpen,
}: {
  phase: 'browse' | 'importing' | 'done'
  selectedCount: number
  canImport: boolean
  progress: { done: number; total: number; title: string }
  summary: ImportSummary | null
  onCancel: () => void
  onImport: () => void
  onImportMore: () => void
  onRetryFailed: () => void
  onOpen: () => void
}) {
  if (phase === 'importing') {
    const percent =
      progress.total === 0
        ? 0
        : Math.round((progress.done / progress.total) * 100)
    return (
      <div className="flex items-center gap-4" role="status">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-sans text-[12px] font-medium text-ink">
              Importing {Math.min(progress.done + 1, progress.total)} of{' '}
              {progress.total}
            </span>
            <span className="truncate font-mono text-[11px] text-ink-ghost">
              {progress.title}
            </span>
          </div>
          <div className="mt-2 h-[3px] overflow-hidden rounded-full bg-surface">
            <div
              className="iii-ui-motion-control h-[3px] rounded-full bg-accent transition-[width]"
              style={{ width: `${percent}%` }}
            />
          </div>
        </div>
        <span className="shrink-0 font-mono text-[12px] tabular-nums text-ink-faint">
          {progress.done}/{progress.total}
        </span>
      </div>
    )
  }
  if (phase === 'done' && summary) {
    const variant =
      summary.imported === 0 ? 'alert' : summary.failed > 0 ? 'warn' : 'success'
    return (
      <StatusPanel
        role={summary.imported > 0 ? 'status' : 'alert'}
        variant={variant}
        icon={
          variant === 'success' ? (
            <CircleCheck className="h-full w-full" />
          ) : (
            <TriangleAlert className="h-full w-full" />
          )
        }
        headline={summaryText(summary)}
        action={
          <>
            <Button variant="ghost" onClick={onImportMore}>
              Import more
            </Button>
            {summary.lastSessionId ? (
              <Button variant="primary" onClick={onOpen}>
                {summary.imported > 1
                  ? 'Open last imported'
                  : 'Open conversation'}
              </Button>
            ) : (
              <Button variant="primary" onClick={onRetryFailed}>
                Retry failed
              </Button>
            )}
          </>
        }
      />
    )
  }
  return (
    <div className="flex flex-wrap items-center gap-3">
      <div className="flex min-w-[14rem] flex-1 items-center gap-2">
        <Info className="size-4 shrink-0 text-ink-ghost" aria-hidden />
        <p className="font-sans text-[12px] leading-[1.5] text-ink-faint">
          Messages and the commands that ran come across. Reasoning, attachments
          and subagents are left behind.
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button variant="primary" disabled={!canImport} onClick={onImport}>
          {selectedCount === 0
            ? 'Import'
            : selectedCount === 1
              ? 'Import 1 conversation'
              : `Import ${selectedCount} conversations`}
        </Button>
      </div>
    </div>
  )
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
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [openId, setOpenId] = useState<string | null>(null)
  const [preview, setPreview] = useState<ConversationPreview | null>(null)
  const [previewError, setPreviewError] = useState('')
  const [previewLoading, setPreviewLoading] = useState(false)
  const [phase, setPhase] = useState<'browse' | 'importing' | 'done'>('browse')
  const [status, setStatus] = useState<Record<string, RowStatus>>({})
  const [failures, setFailures] = useState<Record<string, string>>({})
  const [progress, setProgress] = useState({ done: 0, total: 0, title: '' })
  const [summary, setSummary] = useState<ImportSummary | null>(null)
  const busy = phase === 'importing'

  // Search as you type: the server filters, so wait for the typing to settle.
  useEffect(() => {
    const timer = window.setTimeout(
      () => setSearch(query.trim()),
      SEARCH_DEBOUNCE_MS,
    )
    return () => window.clearTimeout(timer)
  }, [query])

  // biome-ignore lint/correctness/useExhaustiveDependencies: retry explicitly reruns the same discovery request.
  useEffect(() => {
    generation.current += 1
    if (!open) return
    let cancelled = false
    setLoading(true)
    // A new search keeps the current list on screen until the next one lands;
    // a new source (or a reopened dialog) starts from nothing.
    setDiscovery((value) => (value?.source === source ? value : null))
    setSelected(new Set())
    setStatus({})
    setFailures({})
    setSummary(null)
    setPhase('browse')
    setError('')
    void discoverConversations({ source, query: search })
      .then((value) => {
        if (cancelled) return
        setDiscovery(value)
        setOpenId((current) =>
          current && value.conversations.some((c) => c.id === current)
            ? current
            : (value.conversations[0]?.id ?? null),
        )
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
    if (!openId || !open) {
      setPreviewLoading(false)
      return
    }
    let cancelled = false
    setPreviewLoading(true)
    void previewConversation(source, openId)
      .then((value) => {
        if (!cancelled) setPreview(value)
      })
      .catch((e) => {
        if (!cancelled) setPreviewError(errText(e))
      })
      .finally(() => {
        if (!cancelled) setPreviewLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [source, openId, open])

  const conversations = discovery?.conversations ?? []
  const openConversation = conversations.find((c) => c.id === openId) ?? null
  const selectedIds = conversations
    .filter((c) => selected.has(c.id))
    .map((c) => c.id)
  const allSelected =
    conversations.length > 0 && selectedIds.length === conversations.length

  function switchSource(next: ConversationSource) {
    if (busy || next === source) return
    setQuery('')
    setSearch('')
    setOpenId(null)
    setSource(next)
  }

  function toggle(id: string) {
    if (busy) return
    setSelected((value) => {
      const next = new Set(value)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  function toggleAll() {
    if (busy) return
    setSelected(
      allSelected ? new Set() : new Set(conversations.map((c) => c.id)),
    )
  }

  async function loadMore() {
    if (!discovery?.next_cursor || loading) return
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

  async function importIds(ids: string[]) {
    if (ids.length === 0) return
    const titles = new Map(conversations.map((c) => [c.id, c.title || c.id]))
    setPhase('importing')
    setSummary(null)
    setFailures({})
    setStatus(Object.fromEntries(ids.map((id) => [id, 'queued' as const])))
    let imported = 0
    let messages = 0
    let commands = 0
    let failed = 0
    let lastSessionId: string | null = null
    for (const [index, id] of ids.entries()) {
      setProgress({
        done: index,
        total: ids.length,
        title: titles.get(id) ?? '',
      })
      setStatus((value) => ({ ...value, [id]: 'importing' }))
      try {
        const result = await importConversation(source, id)
        imported += 1
        messages += result.total_messages
        commands += result.imported_commands ?? 0
        lastSessionId = result.session_id
        setStatus((value) => ({ ...value, [id]: 'done' }))
      } catch (e) {
        failed += 1
        setFailures((value) => ({ ...value, [id]: errText(e) }))
        setStatus((value) => ({ ...value, [id]: 'failed' }))
      }
    }
    setProgress({ done: ids.length, total: ids.length, title: '' })
    // The batch is over: say so once, for the whole selection. Rows can be
    // scrolled out of view, and nothing else in the console announces a
    // conversation created behind this dialog.
    setSummary({ imported, messages, commands, failed, lastSessionId })
    setPhase('done')
  }

  function importMore() {
    setPhase('browse')
    setSummary(null)
    setStatus({})
    setFailures({})
    setSelected(new Set())
  }

  const failedIds = Object.entries(status)
    .filter(([, value]) => value === 'failed')
    .map(([id]) => id)

  const now = Date.now()
  const showSkeleton = loading && !discovery
  const unavailable = discovery !== null && !discovery.available
  const noResults =
    discovery?.available === true && conversations.length === 0 && !loading

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!busy) onOpenChange(next)
      }}
    >
      <DialogContent className="flex max-h-[90dvh] w-[calc(100%-2rem)] max-w-[960px] flex-col gap-0 overflow-hidden p-0">
        <div className="px-6 pt-[22px]">
          <DialogTitle>Import conversations</DialogTitle>
          <DialogDescription className="mt-1.5 max-w-[70ch] leading-[1.6]">
            Bring Codex and Claude Code histories from this machine into ADE as
            new conversations you can continue.
          </DialogDescription>
        </div>

        <div className="flex flex-wrap items-center gap-2.5 px-6 pt-4">
          <SegmentedControl
            variant="radio"
            aria-label="History source"
            value={source}
            onChange={switchSource}
            options={SOURCE_OPTIONS}
          />
          <SearchField
            className="min-w-[200px] flex-1"
            name="import-search"
            aria-label="Search titles and projects"
            placeholder="Search titles and projects"
            value={query}
            onChange={setQuery}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                setSearch(query.trim())
              }
            }}
            disabled={busy}
          />
          {discovery?.available ? (
            <span className="shrink-0 font-mono text-[11px] tabular-nums text-ink-ghost">
              {loading ? (
                <Loader2
                  className="inline size-4 animate-spin motion-reduce:animate-none"
                  aria-label="Searching"
                />
              ) : (
                `${conversations.length}${discovery.next_cursor ? '+' : ''} found`
              )}
            </span>
          ) : null}
        </div>

        {discovery ? (
          <div className="flex items-center gap-[7px] px-6 pt-2.5">
            <Folder className="size-4 shrink-0 text-ink-ghost" aria-hidden />
            <span className="truncate font-mono text-[11px] text-ink-ghost">
              {discovery.directory}
            </span>
            <span className="shrink-0 font-sans text-[11px] text-ink-ghost">
              on the ADE host
            </span>
          </div>
        ) : null}

        <div className="@container flex min-h-0 flex-col px-6 pt-3.5">
          {error ? (
            <div className="py-6">
              <DiscoveryFailed
                error={error}
                onRetry={() => setRetry((value) => value + 1)}
              />
            </div>
          ) : unavailable ? (
            <SourceUnavailable
              source={source}
              onSwitch={switchSource}
              onRetry={() => setRetry((value) => value + 1)}
            />
          ) : (
            <div className="grid h-[470px] max-h-[55dvh] min-h-0 shrink grid-cols-[minmax(0,1fr)] grid-rows-[minmax(0,1fr)_minmax(0,1fr)] gap-4 @2xl:grid-cols-[minmax(0,348px)_minmax(0,1fr)] @2xl:grid-rows-[minmax(0,1fr)]">
              <div className="flex min-h-0 flex-col">
                <div className="flex h-7 shrink-0 items-center gap-2.5">
                  {showSkeleton ? (
                    <>
                      <Skeleton className="size-[18px]" />
                      <Skeleton className="block h-[9px] w-32" />
                      <span className="ml-auto flex items-center gap-1.5 font-sans text-[11px] text-ink-faint">
                        <Loader2
                          className="size-4 animate-spin text-accent motion-reduce:animate-none"
                          aria-hidden
                        />
                        Reading histories
                      </span>
                    </>
                  ) : (
                    <>
                      <Checkbox
                        className="shrink-0"
                        checked={allSelected}
                        disabled={busy || conversations.length === 0}
                        aria-label="Select all"
                        onChange={toggleAll}
                      />
                      <span className="min-w-0 flex-1 truncate font-sans text-[11px] text-ink-faint">
                        {selectedIds.length === 0
                          ? 'Select conversations to import'
                          : selectedIds.length === 1
                            ? '1 conversation selected'
                            : `${selectedIds.length} conversations selected`}
                      </span>
                      {selectedIds.length > 0 && !busy ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setSelected(new Set())}
                        >
                          Clear
                        </Button>
                      ) : null}
                    </>
                  )}
                </div>
                <section
                  aria-label="Available histories"
                  className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto pr-1"
                >
                  {discovery?.warnings.map((warning) => (
                    <StatusPanel
                      key={warning}
                      variant="warn"
                      className="mb-1"
                      icon={<TriangleAlert className="h-full w-full" />}
                      headline={warning}
                    />
                  ))}
                  {showSkeleton ? (
                    <>
                      <SkeletonRow />
                      <SkeletonRow />
                      <SkeletonRow faded={0.66} />
                      <SkeletonRow faded={0.36} />
                    </>
                  ) : null}
                  {noResults ? (
                    <EmptyState
                      compact
                      title={search ? 'No matches' : 'No conversations'}
                      description={
                        search
                          ? 'No conversation title or project matches this search.'
                          : 'This history has no conversations yet.'
                      }
                    />
                  ) : null}
                  {conversations.map((conversation) => (
                    <HistoryRow
                      key={conversation.id}
                      conversation={conversation}
                      selected={selected.has(conversation.id)}
                      open={conversation.id === openId}
                      status={status[conversation.id]}
                      failure={failures[conversation.id]}
                      busy={busy}
                      now={now}
                      onToggle={() => toggle(conversation.id)}
                      onOpen={() => setOpenId(conversation.id)}
                    />
                  ))}
                  {discovery?.next_cursor ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="mt-0.5 w-full bg-surface text-[12px]"
                      disabled={loading || busy}
                      onClick={() => void loadMore()}
                    >
                      {loading ? (
                        <Loader2
                          className="animate-spin motion-reduce:animate-none"
                          aria-hidden
                        />
                      ) : null}
                      Load more
                    </Button>
                  ) : null}
                </section>
              </div>
              <PreviewPane
                source={source}
                conversation={openConversation}
                preview={preview}
                error={previewError}
                loading={previewLoading || showSkeleton}
              />
            </div>
          )}
        </div>

        <div className="px-6 pt-4 pb-5">
          <ImportFooter
            phase={phase}
            selectedCount={selectedIds.length}
            canImport={selectedIds.length > 0 && !loading && !error}
            progress={progress}
            summary={summary}
            onCancel={() => onOpenChange(false)}
            onImport={() => void importIds(selectedIds)}
            onImportMore={importMore}
            onRetryFailed={() => void importIds(failedIds)}
            onOpen={() => {
              const id = summary?.lastSessionId
              if (id) onImported(id)
              onOpenChange(false)
            }}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}
