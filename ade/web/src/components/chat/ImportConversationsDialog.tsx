import {
  ArrowLeft,
  Bot,
  Check,
  CircleCheck,
  Copy,
  FilePen,
  FileText,
  Folder,
  FolderX,
  Globe,
  Info,
  ListChecks,
  Loader2,
  type LucideIcon,
  MessagesSquare,
  Plug,
  RotateCw,
  Search,
  SearchX,
  SquareTerminal,
  TriangleAlert,
  Wrench,
} from 'lucide-react'
import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { Checkbox } from '@/components/ui/Checkbox'
import { Hint } from '@/components/ui/Hint'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import { SearchField } from '@/components/ui/SearchField'
import { Skeleton } from '@/components/ui/Skeleton'
import { copyTextToClipboard } from '@/lib/clipboard'
import {
  type ConversationPreview,
  type ConversationSource,
  conversationSources,
  conversationTitle,
  type Discovery,
  discoverConversations,
  discoveryNotices,
  type ExternalConversation,
  importConversation,
  type PreviewMessage,
  previewConversation,
  previewNotices,
  projectName,
  promptText,
  titleFromPrompt,
} from '@/lib/conversation-import'
import { errText } from '@/lib/errors'
import { formatElapsed } from '@/lib/relative-time'
import { cn } from '@/lib/utils'

/*
 * Import conversations: one calm column of choices and a quiet preview
 * beside it, in the shape of an importer (Linear, Arc's "import from
 * browser"). Pick a source, tick conversations, read enough of the one in
 * focus to recognise it, import. Notices that are true of every history
 * (what the import leaves behind, the preview window, unreadable files) are
 * said once, in muted type; warn tone is kept for a real problem with the
 * history on screen.
 *
 * The body is a container: from @2xl it is list + preview side by side,
 * below that (a phone, a narrow pane) the preview is a second step with a
 * way back.
 */

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

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`
}

export function summaryText(value: ImportSummary): string {
  const failures = value.failed === 0 ? '' : ` · ${value.failed} failed`
  if (value.imported === 0) return `Nothing imported${failures}`
  const commands =
    value.commands === 0 ? '' : ` · ${plural(value.commands, 'command')}`
  return `Imported ${plural(value.imported, 'conversation')} · ${plural(value.messages, 'message')}${commands}${failures}`
}

/** "2h ago" for a source timestamp, in the sidebar's own elapsed copy. */
export function whenLabel(timestamp: number, now = Date.now()): string {
  const elapsed = formatElapsed(timestamp, now)
  if (elapsed === null) return ''
  return elapsed === 'just now' ? elapsed : `${elapsed} ago`
}

/** `~/.codex` for a directory under the host user's home; anything else as is. */
export function homePath(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, '~')
}

/**
 * The list with the next page appended. The server pages by offset into a list ordered by
 * modification time, so a history that is still being written can shift a conversation onto
 * the following page: keep each one once.
 */
export function mergePages(current: Discovery, page: Discovery): Discovery {
  const known = new Set(current.conversations.map((c) => c.id))
  return {
    ...page,
    skipped: (current.skipped ?? 0) + (page.skipped ?? 0),
    conversations: [
      ...current.conversations,
      ...page.conversations.filter((c) => !known.has(c.id)),
    ],
  }
}

function basename(path: string): string {
  return (
    path
      .split(/[\\/]+/)
      .filter(Boolean)
      .pop() ?? path
  )
}

function firstString(
  input: Record<string, unknown>,
  keys: readonly string[],
): string {
  for (const key of keys) {
    const value = input[key]
    if (typeof value === 'string' && value.trim()) return value.trim()
  }
  return ''
}

export type CallKind =
  | 'run'
  | 'read'
  | 'edit'
  | 'search'
  | 'fetch'
  | 'delegate'
  | 'plan'
  | 'mcp'
  | 'other'

/**
 * One call as a single quiet line: what kind of thing it did, a verb, and
 * the argument that says the most (the command, the file, the query).
 */
export function callSummary(
  functionId: string,
  input: unknown,
): { kind: CallKind; verb: string; detail: string } {
  const args =
    input !== null && typeof input === 'object' && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {}
  const id = functionId.toLowerCase()
  if (['exec', 'bash', 'shell', 'local_shell', 'exec_command'].includes(id)) {
    // Agents prefix most commands with a `cd` into the project: the rest is the command.
    const command = firstString(args, ['command', 'cmd'])
      .replace(/^cd\s+("[^"]*"|'[^']*'|\S+)\s*(?:&&|;)\s*/, '')
      .split('\n')[0]
      ?.trim()
    return command
      ? { kind: 'run', verb: 'Ran', detail: command }
      : { kind: 'run', verb: 'Ran a command', detail: '' }
  }
  if (['read', 'read_file', 'view', 'notebookread'].includes(id)) {
    const path = firstString(args, ['file_path', 'path', 'notebook_path'])
    return path
      ? { kind: 'read', verb: 'Read', detail: basename(path) }
      : { kind: 'read', verb: 'Read a file', detail: '' }
  }
  if (
    [
      'edit',
      'write',
      'multiedit',
      'notebookedit',
      'apply_patch',
      'str_replace',
    ].includes(id)
  ) {
    let path = firstString(args, ['file_path', 'path', 'notebook_path'])
    let more = 0
    const changes = args.changes
    if (!path && changes && typeof changes === 'object') {
      const paths = Array.isArray(changes)
        ? changes
            .map((change) =>
              change && typeof change === 'object'
                ? firstString(change as Record<string, unknown>, ['path'])
                : '',
            )
            .filter(Boolean)
        : Object.keys(changes)
      path = paths[0] ?? ''
      more = Math.max(0, paths.length - 1)
    }
    const detail = path
      ? `${basename(path)}${more > 0 ? ` and ${plural(more, 'more file', 'more files')}` : ''}`
      : ''
    return { kind: 'edit', verb: detail ? 'Edited' : 'Edited files', detail }
  }
  if (['grep', 'glob', 'web_search', 'websearch', 'search'].includes(id)) {
    return {
      kind: 'search',
      verb: 'Searched',
      detail: firstString(args, ['pattern', 'query', 'q']),
    }
  }
  if (['webfetch', 'fetch', 'web_fetch'].includes(id)) {
    return {
      kind: 'fetch',
      verb: 'Fetched',
      detail: firstString(args, ['url']),
    }
  }
  if (['task', 'agent', 'spawn_agent'].includes(id)) {
    return {
      kind: 'delegate',
      verb: 'Delegated',
      detail: firstString(args, ['description', 'prompt']),
    }
  }
  if (['update_plan', 'todowrite'].includes(id)) {
    return { kind: 'plan', verb: 'Updated the plan', detail: '' }
  }
  const mcp =
    /^mcp__(.+?)__(.+)$/.exec(functionId) ?? /^(.+?)::(.+)$/.exec(functionId)
  if (mcp) {
    return { kind: 'mcp', verb: mcp[2], detail: mcp[1] }
  }
  const detail = Object.values(args).find(
    (value): value is string => typeof value === 'string' && !!value.trim(),
  )
  return { kind: 'other', verb: functionId, detail: detail?.trim() ?? '' }
}

const CALL_ICON: Record<CallKind, LucideIcon> = {
  run: SquareTerminal,
  read: FileText,
  edit: FilePen,
  search: Search,
  fetch: Globe,
  delegate: Bot,
  plan: ListChecks,
  mcp: Plug,
  other: Wrench,
}

export type PreviewRow =
  | { kind: 'text'; id: string; role: 'user' | 'assistant'; text: string }
  | {
      kind: 'call'
      id: string
      functionId: string
      call: CallKind
      verb: string
      detail: string
      failed: boolean
    }

/**
 * The preview in reading order: text turns, and one call row per call placed
 * where the call happened, carrying the recorded result's outcome. A result
 * whose call is missing from the window still shows as a row. A prompt shows
 * only the person's own words; a turn that was all injected context is left
 * out.
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
      const functionId = message.function_id ?? 'tool'
      const summary = callSummary(functionId, null)
      rows.push({
        kind: 'call',
        id: message.id,
        functionId,
        call: summary.kind,
        verb: summary.verb,
        detail: summary.detail,
        failed: message.is_error === true,
      })
      continue
    }
    const text =
      message.role === 'user' ? promptText(message.text) : message.text.trim()
    if (text) {
      rows.push({ kind: 'text', id: message.id, role: message.role, text })
    }
    for (const call of message.calls ?? []) {
      claimed.add(call.id)
      const summary = callSummary(call.function_id, call.arguments)
      rows.push({
        kind: 'call',
        id: `${message.id}:${call.id}`,
        functionId: call.function_id,
        call: summary.kind,
        verb: summary.verb,
        detail: summary.detail,
        failed: results.get(call.id)?.is_error === true,
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

/**
 * The preview's title: the listed one when it reads as words, else the
 * first prompt of a preview that starts at the beginning, else the project
 * and day the listing fell back to.
 */
export function previewTitle(
  conversation: ExternalConversation,
  preview: ConversationPreview | null,
): string {
  if (titleFromPrompt(conversation.title))
    return conversationTitle(conversation)
  if (preview && !previewNotices(preview).truncated) {
    for (const message of preview.messages) {
      if (message.role !== 'user') continue
      const title = titleFromPrompt(message.text)
      if (title) return title
    }
  }
  return conversationTitle(conversation)
}

type RowStatus = 'queued' | 'importing' | 'done' | 'failed'

function RowStatusLabel({ status }: { status: RowStatus }) {
  if (status === 'importing') {
    return (
      <span className="flex items-center gap-1.5 text-[13px] text-ink-faint">
        <Loader2
          className="size-4 animate-spin motion-reduce:animate-none"
          aria-hidden
        />
        Importing
      </span>
    )
  }
  if (status === 'done') {
    return (
      <span className="flex items-center gap-1.5 text-[13px] text-ok">
        <CircleCheck className="size-4" aria-hidden />
        Imported
      </span>
    )
  }
  if (status === 'failed') {
    return (
      <span className="flex items-center gap-1.5 text-[13px] text-alert">
        <TriangleAlert className="size-4" aria-hidden />
        Failed
      </span>
    )
  }
  return <span className="text-[13px] text-ink-faint">Queued</span>
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
  onKeyDown,
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
  onKeyDown?: (event: KeyboardEvent<HTMLButtonElement>) => void
}) {
  const title = conversationTitle(conversation)
  const project = projectName(conversation.cwd)
  const meta = [
    // A title that fell back to the project already names it.
    project && !title.startsWith(`${project} · `) ? project : null,
    whenLabel(conversation.updated_at, now),
    conversation.message_count === undefined
      ? null
      : plural(conversation.message_count, 'message'),
  ].filter(Boolean)
  return (
    <div
      data-import-row={conversation.id}
      data-selected={selected || undefined}
      data-open={open || undefined}
      data-status={status}
      className={cn(
        'iii-ui-motion-control group/row flex items-start gap-3 rounded-lg px-3 py-2.5 transition-[background-color] hover:bg-surface-hover has-[[data-row-button]:focus-visible]:ring-2 has-[[data-row-button]:focus-visible]:ring-rule-focus',
        open && 'bg-surface-selected hover:bg-surface-selected',
      )}
    >
      {/* The box only selects; the rest of the row opens the preview. */}
      <Checkbox
        className="mt-px shrink-0"
        checked={selected}
        disabled={busy}
        aria-label={`Select ${title}`}
        onChange={onToggle}
      />
      <button
        type="button"
        data-row-button
        tabIndex={open ? 0 : -1}
        onClick={onOpen}
        onKeyDown={onKeyDown}
        aria-current={open || undefined}
        className="min-w-0 flex-1 text-left outline-none"
      >
        <span className="block truncate font-inter text-sm leading-5 text-ink">
          {title}
        </span>
        <span className="mt-0.5 block truncate font-inter text-[13px] leading-5 text-ink-faint tabular-nums">
          {meta.join(' · ')}
        </span>
        {failure ? (
          <span className="mt-0.5 block truncate font-inter text-[13px] leading-5 text-alert">
            {failure}
          </span>
        ) : null}
      </button>
      {status ? (
        <span className="mt-0.5 shrink-0">
          <RowStatusLabel status={status} />
        </span>
      ) : null}
    </div>
  )
}

function SkeletonRows() {
  return (
    <div aria-hidden className="flex flex-col">
      {[0.82, 0.64, 0.74, 0.56, 0.68].map((width, index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: a fixed placeholder list.
          key={index}
          className="flex items-start gap-3 px-3 py-2.5"
          style={{ opacity: 1 - index * 0.16 }}
        >
          <Skeleton className="size-[18px] rounded-[5px]" />
          <div className="min-w-0 flex-1 pt-0.5">
            <Skeleton
              className="block h-[12px] rounded-sm"
              style={{ width: `${width * 100}%` }}
            />
            <Skeleton className="mt-2 block h-[10px] w-2/5 rounded-sm opacity-70" />
          </div>
        </div>
      ))}
    </div>
  )
}

function PreviewSkeleton() {
  return (
    <div aria-hidden className="flex flex-col gap-5">
      <div className="ml-auto w-3/5 rounded-xl bg-surface px-3.5 py-3">
        <Skeleton className="block h-[10px] w-full rounded-sm" />
        <Skeleton className="mt-2 block h-[10px] w-2/3 rounded-sm" />
      </div>
      <div className="flex flex-col gap-2">
        <Skeleton className="block h-[10px] w-full rounded-sm" />
        <Skeleton className="block h-[10px] w-11/12 rounded-sm" />
        <Skeleton className="block h-[10px] w-3/5 rounded-sm" />
      </div>
      <div className="flex flex-col gap-2.5">
        <Skeleton className="block h-[10px] w-1/2 rounded-sm opacity-70" />
        <Skeleton className="block h-[10px] w-2/5 rounded-sm opacity-70" />
      </div>
    </div>
  )
}

/** A centred placeholder: a soft glyph tile, one line of title, one of help, actions. */
function Placeholder({
  icon: Icon,
  tone = 'quiet',
  title,
  children,
  actions,
  role,
}: {
  icon: LucideIcon
  tone?: 'quiet' | 'alert'
  title: string
  children?: ReactNode
  actions?: ReactNode
  role?: 'alert' | 'status'
}) {
  return (
    <div
      role={role}
      className="mx-auto flex h-full w-full max-w-sm flex-col items-center justify-center px-4 py-8 text-center font-inter"
    >
      <span
        className={cn(
          'mb-3 flex size-10 items-center justify-center rounded-xl',
          tone === 'alert'
            ? 'bg-alert-muted text-alert'
            : 'bg-muted text-ink-faint',
        )}
      >
        <Icon className="size-5" aria-hidden />
      </span>
      <p className="text-sm font-medium text-ink">{title}</p>
      {children ? (
        <div className="mt-1 w-full text-[13px] leading-5 text-ink-faint">
          {children}
        </div>
      ) : null}
      {actions ? (
        <div className="mt-4 flex flex-wrap justify-center gap-2">
          {actions}
        </div>
      ) : null}
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
  if (role === 'user') {
    return (
      <article
        data-preview-role="user"
        aria-label="You"
        className="my-3 ml-auto w-fit max-w-[85%] rounded-xl bg-surface px-3.5 py-2.5"
      >
        <p className="line-clamp-6 whitespace-pre-wrap break-words font-inter text-sm leading-6 text-ink">
          {text}
        </p>
      </article>
    )
  }
  return (
    <article
      data-preview-role="assistant"
      aria-label={sourceLabel}
      className="my-3"
    >
      <p className="line-clamp-8 whitespace-pre-wrap break-words font-inter text-sm leading-6 text-ink">
        {text}
      </p>
    </article>
  )
}

/** One call as a quiet line: glyph, verb, the argument that matters in mono. */
function CallRow({
  functionId,
  call,
  verb,
  detail,
  failed,
}: {
  functionId: string
  call: CallKind
  verb: string
  detail: string
  failed: boolean
}) {
  const Icon = CALL_ICON[call]
  return (
    <div
      data-preview-role="function-call"
      data-function-id={functionId}
      data-function-status={failed ? 'error' : 'done'}
      className="flex h-7 min-w-0 items-center gap-2 font-inter text-[13px] text-ink-faint"
    >
      {/* A failed call only tints its glyph: shells exit non-zero all the
          time, and a column of red labels would outshout the transcript. */}
      <Icon
        className={cn('size-4 shrink-0', failed && 'text-alert/80')}
        aria-hidden
      />
      <span className="shrink-0">{verb}</span>
      {detail ? (
        <span className="min-w-0 truncate font-geist-mono text-xs">
          {detail}
        </span>
      ) : null}
      {failed ? <span className="sr-only">(failed)</span> : null}
    </div>
  )
}

export function PreviewPane({
  source,
  conversation,
  preview,
  error,
  loading,
  selected,
  busy,
  onToggle,
  onBack,
}: {
  source: ConversationSource
  conversation: ExternalConversation | null
  preview: ConversationPreview | null
  error: string
  loading: boolean
  selected?: boolean
  busy?: boolean
  onToggle?: () => void
  onBack?: () => void
}) {
  const sourceLabel = conversationSources[source]
  const rows = useMemo(
    () => (preview ? previewRows(preview.messages) : []),
    [preview],
  )
  const notices = preview ? previewNotices(preview) : null
  const shown = preview ? previewCounts(preview.messages) : null
  const total =
    conversation?.message_count !== undefined
      ? {
          messages: conversation.message_count,
          commands: conversation.command_count ?? 0,
        }
      : shown && !notices?.truncated
        ? shown
        : null
  const title = conversation ? previewTitle(conversation, preview) : ''
  const project = projectName(conversation?.cwd)
  return (
    <section
      aria-label="Preview"
      className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl bg-card ring-1 ring-foreground/10"
    >
      {conversation ? (
        <header className="shrink-0 border-b border-border px-4 pt-3.5 pb-3">
          <div className="flex items-start gap-2">
            {onBack ? (
              <Hint label="Back to conversations">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="-mt-0.5 -ml-1.5 shrink-0 @2xl:hidden"
                  aria-label="Back to conversations"
                  onClick={onBack}
                >
                  <ArrowLeft />
                </Button>
              </Hint>
            ) : null}
            <h3 className="line-clamp-2 min-w-0 flex-1 font-inter text-[15px] font-semibold leading-6 tracking-[-0.01em] text-ink">
              {title}
            </h3>
            {onToggle ? (
              // The list's own box is out of sight on the preview step.
              <span className="mt-0.5 shrink-0 @2xl:hidden">
                <Checkbox
                  checked={selected === true}
                  disabled={busy}
                  aria-label={`Select ${title}`}
                  onChange={onToggle}
                />
              </span>
            ) : null}
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 font-inter text-[13px] leading-5 text-ink-faint">
            <Hint label={conversation.cwd ?? 'No project recorded'}>
              <span className="inline-flex max-w-[16rem] items-center gap-1.5 rounded-md bg-surface px-1.5 py-0.5 text-xs font-medium text-ink-faint">
                <Folder className="size-4 shrink-0" aria-hidden />
                <span className="truncate">{project ?? 'No project'}</span>
              </span>
            </Hint>
            <span className="tabular-nums">
              {new Date(conversation.updated_at).toLocaleString(undefined, {
                month: 'short',
                day: 'numeric',
                hour: 'numeric',
                minute: '2-digit',
              })}
            </span>
            {total ? (
              <>
                <span aria-hidden>·</span>
                <span className="tabular-nums">
                  {plural(total.messages, 'message')}
                  {total.commands > 0
                    ? ` · ${plural(total.commands, 'command')}`
                    : ''}
                </span>
              </>
            ) : null}
          </div>
        </header>
      ) : null}
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {!conversation ? (
          <Placeholder icon={MessagesSquare} title="Nothing to preview">
            Choose a conversation to read the end of it here.
          </Placeholder>
        ) : error ? (
          <Placeholder
            icon={TriangleAlert}
            tone="alert"
            title="Could not read this conversation"
          >
            <span className="break-all font-geist-mono text-xs">{error}</span>
          </Placeholder>
        ) : loading || !preview || !notices || !shown ? (
          <PreviewSkeleton />
        ) : (
          <>
            {notices.truncated ? (
              <p className="mb-1 text-center font-inter text-xs text-ink-faint tabular-nums">
                {conversation.message_count !== undefined
                  ? `Showing the last ${shown.messages} of ${plural(conversation.message_count, 'message')}`
                  : 'Showing the latest activity'}
              </p>
            ) : null}
            {notices.problems.map((problem) => (
              <p
                key={problem}
                className="my-2 flex items-start gap-2 font-inter text-[13px] leading-5 text-warn"
              >
                <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
                {problem}
              </p>
            ))}
            {rows.length === 0 ? (
              <Placeholder icon={MessagesSquare} title="Nothing to import">
                This conversation has no messages or commands to bring across.
              </Placeholder>
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
                  call={row.call}
                  verb={row.verb}
                  detail={row.detail}
                  failed={row.failed}
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
  directory,
  onSwitch,
  onRetry,
}: {
  source: ConversationSource
  directory?: string
  onSwitch: (next: ConversationSource) => void
  onRetry: () => void
}) {
  const other = SOURCE_OPTIONS.find((option) => option.value !== source)
  return (
    <Placeholder
      icon={FolderX}
      title={`No ${conversationSources[source]} history on this machine`}
      actions={
        <>
          {other ? (
            <Button variant="secondary" onClick={() => onSwitch(other.value)}>
              Try {other.label}
            </Button>
          ) : null}
          <Button variant="secondary" onClick={onRetry}>
            Check again
          </Button>
        </>
      }
    >
      ADE looked in{' '}
      {directory ? (
        <span className="font-geist-mono text-xs text-ink">
          {homePath(directory)}
        </span>
      ) : (
        'its history directory'
      )}{' '}
      and found nothing to read. When ADE runs in a container, that directory
      has to be mounted into it.
    </Placeholder>
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
    <Placeholder
      role="alert"
      icon={TriangleAlert}
      tone="alert"
      title="Could not read the history directory"
      actions={
        <>
          <Button variant="secondary" onClick={onRetry}>
            <RotateCw aria-hidden />
            Retry
          </Button>
          <Button
            variant="secondary"
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
        </>
      }
    >
      Nothing was imported or changed.
      <code className="mt-3 block break-all rounded-lg bg-surface px-3 py-2 text-left font-geist-mono text-xs leading-5 text-ink-faint">
        {error}
      </code>
    </Placeholder>
  )
}

const LEFT_BEHIND =
  'Messages and commands come across; reasoning, attachments and subagents stay behind.'

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
      <div className="flex items-center gap-4 font-inter" role="status">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-baseline gap-2 text-sm">
            <span className="shrink-0 font-medium text-ink tabular-nums">
              Importing {Math.min(progress.done + 1, progress.total)} of{' '}
              {progress.total}
            </span>
            <span className="truncate text-[13px] text-ink-faint">
              {progress.title}
            </span>
          </div>
          <div className="mt-2 h-1 overflow-hidden rounded-full bg-surface">
            <div
              className="iii-ui-motion-control h-1 rounded-full bg-primary transition-[width]"
              style={{ width: `${percent}%` }}
            />
          </div>
        </div>
      </div>
    )
  }
  if (phase === 'done' && summary) {
    const outcome =
      summary.imported === 0 ? 'none' : summary.failed > 0 ? 'partial' : 'all'
    return (
      <div className="flex flex-wrap items-center gap-x-4 gap-y-3 font-inter">
        <p
          role={outcome === 'none' ? 'alert' : 'status'}
          className="flex min-w-[14rem] flex-1 items-center gap-2 text-sm text-ink"
        >
          {outcome === 'all' ? (
            <CircleCheck className="size-4 shrink-0 text-ok" aria-hidden />
          ) : (
            <TriangleAlert
              className={cn(
                'size-4 shrink-0',
                outcome === 'none' ? 'text-alert' : 'text-warn',
              )}
              aria-hidden
            />
          )}
          {summaryText(summary)}
        </p>
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="secondary" onClick={onImportMore}>
            Import more
          </Button>
          {summary.lastSessionId ? (
            <Button onClick={onOpen}>
              {summary.imported > 1
                ? 'Open last imported'
                : 'Open conversation'}
            </Button>
          ) : (
            <Button onClick={onRetryFailed}>Retry failed</Button>
          )}
        </div>
      </div>
    )
  }
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-3 font-inter">
      <p className="flex min-w-[16rem] flex-1 items-start gap-2 text-[13px] leading-5 text-ink-faint">
        <Info className="mt-0.5 size-4 shrink-0" aria-hidden />
        {LEFT_BEHIND}
      </p>
      <div className="ml-auto flex shrink-0 items-center gap-2">
        <Button variant="secondary" onClick={onCancel}>
          Cancel
        </Button>
        <Button disabled={!canImport} onClick={onImport}>
          {selectedCount === 0
            ? 'Import'
            : `Import ${plural(selectedCount, 'conversation')}`}
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
  const listRef = useRef<HTMLElement>(null)
  const [discovery, setDiscovery] = useState<Discovery | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [openId, setOpenId] = useState<string | null>(null)
  // Below @2xl the preview is a step of its own; wider, both show.
  const [step, setStep] = useState<'list' | 'preview'>('list')
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
  const someSelected = selectedIds.length > 0 && !allSelected
  const notices = discovery ? discoveryNotices(discovery) : null

  function switchSource(next: ConversationSource) {
    if (busy || next === source) return
    setQuery('')
    setSearch('')
    setOpenId(null)
    setStep('list')
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

  /** ↑/↓ (and Home/End) move the preview through the list; Space ticks the row. */
  function rowKeyDown(id: string, event: KeyboardEvent<HTMLButtonElement>) {
    if (event.key === ' ') {
      event.preventDefault()
      toggle(id)
      return
    }
    const index = conversations.findIndex((c) => c.id === id)
    const target =
      event.key === 'ArrowDown'
        ? index + 1
        : event.key === 'ArrowUp'
          ? index - 1
          : event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? conversations.length - 1
              : null
    if (target === null) return
    event.preventDefault()
    const next =
      conversations[Math.max(0, Math.min(target, conversations.length - 1))]
    if (!next) return
    setOpenId(next.id)
    listRef.current
      ?.querySelector<HTMLButtonElement>(
        `[data-import-row="${CSS.escape(next.id)}"] [data-row-button]`,
      )
      ?.focus()
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
      setDiscovery(mergePages(discovery, page))
    } catch (e) {
      if (generation.current === requestGeneration) setError(errText(e))
    } finally {
      if (generation.current === requestGeneration) setLoading(false)
    }
  }

  async function importIds(ids: string[]) {
    if (ids.length === 0) return
    const titles = new Map(
      conversations.map((c) => [c.id, conversationTitle(c)]),
    )
    setPhase('importing')
    setStep('list')
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
  const countLabel = discovery?.available
    ? `${conversations.length}${discovery.next_cursor ? '+' : ''}`
    : ''

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!busy) onOpenChange(next)
      }}
    >
      <DialogContent className="flex h-[min(720px,calc(100dvh-2rem))] w-[calc(100%-2rem)] max-w-[1000px] [--dialog-max-width:1000px] flex-col gap-0 overflow-hidden p-0">
        <div className="shrink-0 px-5 pt-5 pr-12">
          <DialogTitle className="font-inter text-base font-semibold tracking-[-0.01em] text-ink">
            Import conversations
          </DialogTitle>
          <DialogDescription className="mt-1 font-inter text-sm text-ink-faint">
            Pick up Codex and Claude Code sessions from this machine where they
            left off.
          </DialogDescription>
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-2 px-5 pt-4 pb-3">
          <SegmentedControl
            variant="radio"
            aria-label="History source"
            value={source}
            onChange={switchSource}
            options={SOURCE_OPTIONS}
          />
          <SearchField
            className="min-w-[12rem] flex-1"
            name="import-search"
            aria-label="Search by title or project"
            placeholder="Search by title or project"
            value={query}
            onChange={setQuery}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                setSearch(query.trim())
              }
            }}
            disabled={busy || unavailable}
          />
        </div>

        <div className="@container flex min-h-0 flex-1 flex-col border-t border-border px-5 pt-3">
          {error ? (
            <DiscoveryFailed
              error={error}
              onRetry={() => setRetry((value) => value + 1)}
            />
          ) : unavailable ? (
            <SourceUnavailable
              source={source}
              directory={discovery?.directory}
              onSwitch={switchSource}
              onRetry={() => setRetry((value) => value + 1)}
            />
          ) : noResults ? (
            // Nothing to list: the whole body says so, with the way on.
            search ? (
              <Placeholder
                icon={SearchX}
                title={`No matches for “${search}”`}
                actions={
                  discovery?.next_cursor ? (
                    <Button variant="secondary" onClick={() => void loadMore()}>
                      Search older histories
                    </Button>
                  ) : (
                    <Button variant="secondary" onClick={() => setQuery('')}>
                      Clear search
                    </Button>
                  )
                }
              >
                {discovery?.next_cursor
                  ? 'None of the newest histories match. Older ones have not been searched yet.'
                  : 'Search matches conversation titles and project folders.'}
              </Placeholder>
            ) : (
              <Placeholder
                icon={MessagesSquare}
                title={`No ${conversationSources[source]} conversations yet`}
                actions={SOURCE_OPTIONS.filter((o) => o.value !== source).map(
                  (o) => (
                    <Button
                      key={o.value}
                      variant="secondary"
                      onClick={() => switchSource(o.value)}
                    >
                      Try {o.label}
                    </Button>
                  ),
                )}
              >
                Conversations show up here once {conversationSources[source]}{' '}
                has saved one on this machine.
              </Placeholder>
            )
          ) : (
            <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-4 @2xl:grid-cols-[minmax(0,340px)_minmax(0,1fr)]">
              <div
                data-step={step}
                className="flex min-h-0 flex-col data-[step=preview]:hidden @2xl:data-[step=preview]:flex"
              >
                {/* The select-all box sits on the rows' own checkbox column. */}
                <div className="flex h-9 shrink-0 items-center gap-3 px-3 font-inter">
                  {showSkeleton ? (
                    <>
                      <Skeleton className="size-[18px] rounded-[5px]" />
                      <span className="flex items-center gap-1.5 text-[13px] text-ink-faint">
                        <Loader2
                          className="size-4 animate-spin motion-reduce:animate-none"
                          aria-hidden
                        />
                        Reading {conversationSources[source]} history
                      </span>
                    </>
                  ) : (
                    <>
                      <Checkbox
                        className="shrink-0"
                        checked={allSelected}
                        indeterminate={someSelected}
                        disabled={busy || conversations.length === 0}
                        aria-label="Select all"
                        onChange={toggleAll}
                      />
                      <span className="min-w-0 flex-1 truncate text-[13px] text-ink-faint tabular-nums">
                        {selectedIds.length === 0 ? (
                          'Select all'
                        ) : (
                          <span className="font-medium text-ink">
                            {selectedIds.length} selected
                          </span>
                        )}
                      </span>
                      {selectedIds.length > 0 && !busy ? (
                        <Button
                          variant="ghost"
                          size="xs"
                          className="-mr-1.5 text-ink-faint"
                          onClick={() => setSelected(new Set())}
                        >
                          Clear
                        </Button>
                      ) : loading ? (
                        <Loader2
                          className="size-4 animate-spin text-ink-faint motion-reduce:animate-none"
                          aria-label="Searching"
                        />
                      ) : countLabel ? (
                        <span className="shrink-0 text-[13px] text-ink-faint tabular-nums">
                          {countLabel}
                        </span>
                      ) : null}
                    </>
                  )}
                </div>
                <section
                  ref={listRef}
                  aria-label="Available histories"
                  className="-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1 pb-3"
                >
                  {showSkeleton ? <SkeletonRows /> : null}
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
                      onOpen={() => {
                        setOpenId(conversation.id)
                        setStep('preview')
                      }}
                      onKeyDown={(event) => rowKeyDown(conversation.id, event)}
                    />
                  ))}
                  {discovery?.next_cursor && conversations.length > 0 ? (
                    <Button
                      variant="ghost"
                      className="mt-1 w-full text-ink-faint"
                      disabled={loading || busy}
                      onClick={() => void loadMore()}
                    >
                      {loading ? (
                        <Loader2
                          className="animate-spin motion-reduce:animate-none"
                          aria-hidden
                        />
                      ) : null}
                      Load older conversations
                    </Button>
                  ) : null}
                  {notices?.problems.map((problem) => (
                    <p
                      key={problem}
                      className="mt-2 flex items-start gap-2 px-3 font-inter text-[13px] leading-5 text-warn"
                    >
                      <TriangleAlert
                        className="mt-0.5 size-4 shrink-0"
                        aria-hidden
                      />
                      {problem}
                    </p>
                  ))}
                </section>
                {discovery ? (
                  <p className="flex h-8 shrink-0 items-center gap-1.5 border-t border-border px-3 font-inter text-xs text-ink-faint">
                    <Hint label={`${discovery.directory} on the ADE host`}>
                      <span className="flex min-w-0 items-center gap-1.5">
                        <Folder className="size-4 shrink-0" aria-hidden />
                        <span className="truncate font-geist-mono">
                          {homePath(discovery.directory)}
                        </span>
                      </span>
                    </Hint>
                    {notices && notices.skipped > 0 ? (
                      <Hint label="Empty, unsupported or unreadable histories are left out">
                        <span className="shrink-0 tabular-nums">
                          · {notices.skipped} skipped
                        </span>
                      </Hint>
                    ) : null}
                  </p>
                ) : null}
              </div>
              <div
                data-step={step}
                className="flex min-h-0 flex-col pb-3 data-[step=list]:hidden @2xl:data-[step=list]:flex"
              >
                <PreviewPane
                  source={source}
                  conversation={openConversation}
                  preview={preview}
                  error={previewError}
                  loading={previewLoading || showSkeleton}
                  selected={openId !== null && selected.has(openId)}
                  busy={busy}
                  onToggle={openId ? () => toggle(openId) : undefined}
                  onBack={() => setStep('list')}
                />
              </div>
            </div>
          )}
        </div>

        <div className="shrink-0 border-t border-border px-5 py-3.5">
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
