import {
  Badge,
  Button,
  Chip,
  CodeEditor,
  ConfirmDialog,
  EmptyState,
  IconButton,
  Input,
  Markdown,
  PageHeader,
  PageMain,
  PageShell,
  Panel,
  PanelBody,
  PanelHeader,
  SegmentedControl,
  Select,
  Selector,
  Skeleton,
  StatusDot,
  StatusPanel,
  uiClasses,
  useConfirm,
  type Host,
  type PageRenderProps,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useContainerNarrow, useCopyFlash, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUpRight,
  AtSign,
  Bot,
  Check,
  CirclePlus,
  Clock,
  Columns2,
  Copy,
  Flag,
  ListTree,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Tag,
  Ticket as TicketIcon,
  Trash2,
  TriangleAlert,
  X,
  type LucideIcon,
} from 'lucide-react'
import { Fragment, memo, useCallback, useEffect, useId, useMemo, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { allLabels } from './filters'
import { TICKET_HREF, linkTicketKeys, type LinkedTicket } from './links'
import {
  BOARD_PAGE_ID,
  PLAN_CHANGED_EVENT,
  PROJECTS_PAGE_ID,
  Pills,
  PriorityChip,
  RELEASES_PAGE_ID,
  StatusGlyph,
  TICKET_PAGE_ID,
  api,
  noteOwnWrite,
  priorityLabel,
  statusLabel,
  subscribeChanges,
  type Activity,
  type AgentProfile,
  type BoardData,
  type Brief,
  type Change,
  type Column,
  type Ticket,
  type TicketLinks,
  type TicketSummary,
} from './shared'
import {
  groupSubtaskAdds,
  parentBlockedReason,
  parentOffer,
  parentSentence,
  planSentence,
  subtaskSentence,
  type Part,
} from './subtask-rules'
import {
  DoneWarning,
  ParentSelector,
  ParentStrip,
  ProjectSelector,
  ReleaseSelector,
  SubtasksSection,
  TicketRef,
  type Act,
} from './subtasks'

function readContextId(context: unknown): string | null {
  if (!context || typeof context !== 'object') return null
  const value = (context as { id?: unknown }).id
  return typeof value === 'string' && value.trim() ? value : null
}

/** `15h ago`; formatRelative's `just now` already says it all. */
function ago(time: string): string {
  const relative = formatRelative(time)
  return relative && relative !== 'just now' ? `${relative} ago` : relative
}

const absoluteTime = (time: string) => new Date(time).toLocaleString()

const FIELD_LABELS: Record<string, string> = {
  title: 'the title',
  description: 'the description',
  status: 'the status',
  priority: 'the priority',
  assignee: 'the assignee',
  labels: 'the labels',
}

function describeChange(change: Change, columns: Column[], nameOf: (id: string) => string): string {
  const field = FIELD_LABELS[change.field] ?? change.field
  if (change.field === 'status') {
    return `moved this from ${statusLabel(columns, String(change.from))} to ${statusLabel(columns, String(change.to))}`
  }
  if (change.field === 'assignee') {
    return change.to ? `assigned this to ${nameOf(String(change.to))}` : 'unassigned this'
  }
  return `changed ${field}`
}

/** The strings of a JSON array, each once: they become React keys. */
function stringList(value: unknown): string[] {
  return Array.isArray(value) ? [...new Set(value.filter((item): item is string => typeof item === 'string'))] : []
}

/** The server compares labels with ASCII case ignored; so do the suggestions. */
const asciiLower = (text: string) => text.replace(/[A-Z]/g, (char) => char.toLowerCase())

/** A response the screen must not show: another ticket's, or older than what it shows (a live event can land first). */
function isStale(shown: Ticket | null, next: Ticket): boolean {
  if (!shown) return false
  if (next.id !== shown.id) return true
  const [at, shownAt] = [Date.parse(next.updated_at), Date.parse(shown.updated_at)]
  return at < shownAt || (at === shownAt && next.activity.length < shown.activity.length)
}

/** Whether the ticket's own text (title, description, comments) names `key`. */
function mentionsKey(ticket: Ticket, key: string): boolean {
  const needle = key.toLowerCase()
  const texts = [ticket.title, ticket.description, ...(ticket.comments ?? []).map((comment) => comment.body)]
  return texts.some((text) => text?.toLowerCase().includes(needle))
}

/** The one-line view of a ticket, as a subtask's `parent` carries it. */
function briefOf(ticket: Ticket): Brief {
  const { id, key, title, status, priority, assignee, deleted_at, project_id, release_id } = ticket
  return { id, key, title, status, priority, assignee, deleted_at, project_id, release_id }
}

/** Set when a ticket opens another: the next screen to show a ticket puts focus on its title, the clicked control is gone. */
let focusTitleBefore = 0

/** "added the label [x]", "removed the labels [x] [y]": what the change did, not that it happened. */
function describeLabelChange(change: Change): ReactNode {
  const from = stringList(change.from)
  const to = stringList(change.to)
  const parts = [
    { verb: 'added', labels: to.filter((label) => !from.includes(label)) },
    { verb: 'removed', labels: from.filter((label) => !to.includes(label)) },
  ].filter((part) => part.labels.length > 0)
  if (parts.length === 0) return 'changed the labels'
  return parts.map((part, index) => (
    <Fragment key={part.verb}>
      {index > 0 ? ', ' : null}
      {part.verb} the {part.labels.length === 1 ? 'label' : 'labels'}{' '}
      <span className="kanban-event__chips">
        {part.labels.map((label) => (
          <Chip key={label}>{label}</Chip>
        ))}
      </span>
    </Fragment>
  ))
}

type KeyRef = (key: string) => ReactNode

/** A sentence made of text and ticket keys: a key opens its ticket. */
function renderParts(parts: Part[], ref: KeyRef): ReactNode {
  return parts.map((part, index) =>
    typeof part === 'string' ? part : <Fragment key={`${part.ticket}:${index}`}>{ref(part.ticket)}</Fragment>,
  )
}

function describeField(change: Change, columns: Column[], nameOf: (id: string) => string, ref: KeyRef): ReactNode {
  if (change.field === 'labels') return describeLabelChange(change)
  if (change.field === 'parent') return renderParts(parentSentence(change), ref)
  if (change.field === 'project' || change.field === 'release') return planSentence(change)
  return describeChange(change, columns, nameOf)
}

function activitySentence(entry: Activity, columns: Column[], nameOf: (id: string) => string, ref: KeyRef): ReactNode {
  switch (entry.type) {
    case 'ticket.created':
      return `created this in ${statusLabel(columns, String(entry.changes?.[0]?.to ?? ''))}`
    case 'ticket.moved':
      return entry.changes && entry.changes[0] ? describeChange(entry.changes[0], columns, nameOf) : 'moved this'
    case 'ticket.updated': {
      const changes = entry.changes ?? []
      if (changes.length === 0) return 'updated this'
      return changes.map((change, index) => (
        <Fragment key={`${change.field}:${index}`}>
          {index > 0 ? ', ' : null}
          {describeField(change, columns, nameOf, ref)}
        </Fragment>
      ))
    }
    case 'subtask.added':
    case 'subtask.removed':
      return renderParts(subtaskSentence(entry.type, entry.changes), ref)
    case 'ticket.deleted':
      return 'deleted this ticket'
    case 'ticket.restored':
      return 'restored this ticket'
    case 'comment.created':
      return 'commented'
    default:
      return entry.type
  }
}

/** The glyph for a non-comment entry: what happened, not that something did. */
function activityIcon(entry: Activity): LucideIcon {
  switch (entry.type) {
    case 'ticket.created':
      return CirclePlus
    case 'ticket.moved':
      return ArrowRight
    case 'ticket.deleted':
      return Trash2
    case 'ticket.restored':
      return RotateCcw
    case 'subtask.added':
    case 'subtask.removed':
      return ListTree
    default: {
      const changes = entry.changes ?? []
      const only = (...fields: string[]) =>
        changes.length > 0 && changes.every((change) => fields.includes(change.field))
      return only('labels') ? Tag : only('parent') ? ListTree : only('project', 'release') ? Flag : Pencil
    }
  }
}

type ActivityFilter = 'all' | 'comments' | 'changes'

type LinkRow = { ticket: TicketSummary; relation: 'mentioned here' | 'mentions this' | 'both ways' }

/** One row per ticket, whichever side of the link it is on. */
function mergeLinks(result: TicketLinks): LinkRow[] {
  const rows = new Map<string, LinkRow>()
  for (const ticket of result.mentions ?? []) rows.set(ticket.id, { ticket, relation: 'mentioned here' })
  for (const ticket of result.mentioned_by ?? []) {
    rows.set(ticket.id, { ticket, relation: rows.has(ticket.id) ? 'both ways' : 'mentions this' })
  }
  return [...rows.values()]
}

/** Props of the pane root: `data-narrow` below 720px turns every control into a 44px target (ticket.css). */
function useShell() {
  const { ref, narrow } = useContainerNarrow()
  return { narrow, props: { className: 'kanban-shell', 'data-narrow': narrow ? 'true' : undefined, ref } }
}

function CopyId({ text }: { text: string }) {
  const { state, copy } = useCopyFlash(text)
  return (
    <>
      <IconButton className="kanban-touch" label="Copy ticket id" onClick={() => copy()} variant="ghost">
        {state === 'copied' ? <Check size={16} /> : state === 'failed' ? <TriangleAlert size={16} /> : <Copy size={16} />}
      </IconButton>
      <span className="kanban-copy-status" role="status">
        {state === 'failed' ? 'Copy failed' : ''}
      </span>
    </>
  )
}

/** A description or comment with its ticket keys linked; the rewrite runs once per text, not once per render. */
const Prose = memo(function Prose({
  text,
  tickets,
  onOpen,
}: {
  text: string
  tickets: LinkedTicket[]
  onOpen: (event: MouseEvent<HTMLElement>) => void
}) {
  const markdown = useMemo(() => linkTicketKeys(text, tickets), [text, tickets])
  return (
    <div className="kanban-prose" onAuxClickCapture={onOpen} onClickCapture={onOpen}>
      <Markdown>{markdown}</Markdown>
    </div>
  )
})

type ComposerProps = {
  inputId: string
  replyTo: { id: string; author: string } | null
  busy: boolean
  /** Resolves true once the comment is saved: a failed one keeps what was typed. */
  onSubmit: (body: string) => Promise<boolean>
  onCancelReply: () => void
}

function Composer({ inputId, replyTo, busy, onSubmit, onCancelReply }: ComposerProps) {
  const [body, setBody] = useState('')
  // Another comment to reply to starts a fresh box. Not when the reply is sent: the target resets to null then,
  // and the box may already hold the next comment.
  useEffect(() => {
    if (replyTo) setBody('')
  }, [replyTo?.id])
  const cancelReply = () => {
    setBody('')
    onCancelReply()
  }
  const submit = async () => {
    const trimmed = body.trim()
    if (!trimmed || busy) return
    // What was sent leaves the box, what was typed while the request was in flight stays.
    if (await onSubmit(trimmed)) setBody((now) => (now.startsWith(body) ? now.slice(body.length).trimStart() : now))
  }
  return (
    <div className="kanban-composer">
      {replyTo ? (
        <div className="kanban-composer__reply">
          <span>Replying to {replyTo.author}</span>
          <Button className="kanban-touch" onClick={cancelReply} size="sm" variant="ghost">
            Cancel
          </Button>
        </div>
      ) : null}
      <textarea
        aria-label={replyTo ? 'Reply to comment' : 'Add a comment'}
        className="kanban-composer__input"
        id={inputId}
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && replyTo) {
            cancelReply()
            return
          }
          if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
            event.preventDefault()
            void submit()
          }
        }}
        placeholder={replyTo ? 'Write a reply…' : 'Add a comment…'}
        rows={3}
        value={body}
      />
      <div className="kanban-composer__foot">
        <span className="kanban-composer__hint">Markdown supported · ⌘↵ to send</span>
        <Button
          className="kanban-touch"
          disabled={busy || body.trim().length === 0}
          onClick={() => void submit()}
          size="sm"
          variant="primary"
        >
          {replyTo ? 'Reply' : 'Comment'}
        </Button>
      </div>
    </div>
  )
}

type TicketScreenProps = {
  host: Host
  ticketId: string
  columns: Column[]
  priorities: string[]
  agents: AgentProfile[]
  /** The column that counts as done, when the caller already knows it; the board read overrides it. */
  doneStatus?: string
  /** The same for the canceled column (null = the board has none). */
  canceledStatus?: string | null
  onBack?: () => void
  /** Where a linked ticket opens when the host has no panes (`host.panels`). */
  onOpenTicket?: (key: string) => void
  onRequestClose?: () => void
  onDeleted?: (deleted: boolean) => void
  setDirty?: (dirty: boolean | string) => void
  commands?: PageRenderProps['commands']
}

export function TicketScreen({
  host,
  ticketId,
  columns,
  priorities,
  agents,
  doneStatus: knownDoneStatus,
  canceledStatus: knownCanceledStatus,
  onBack,
  onOpenTicket,
  onRequestClose,
  onDeleted,
  setDirty,
  commands,
}: TicketScreenProps) {
  const [ticket, setTicket] = useState<Ticket | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [confirming, setConfirming] = useState(false)
  const [editingDescription, setEditingDescription] = useState(false)
  const [draft, setDraft] = useState('')
  const [editingTitle, setEditingTitle] = useState(false)
  const [titleDraft, setTitleDraft] = useState('')
  const [replyTo, setReplyTo] = useState<{ id: string; author: string } | null>(null)
  const [pending, setPending] = useState(0)
  const [board, setBoard] = useState<BoardData | null>(null)
  const [parentView, setParentView] = useState<Ticket | null>(null)
  const [links, setLinks] = useState<LinkRow[]>([])
  const [activityFilter, setActivityFilter] = useState<ActivityFilter>('all')
  const { narrow, props: shell } = useShell()
  const requestRef = useRef(0)
  // The ticket on screen, for the callbacks that outlive a render. Callers key this screen by ticket id,
  // so one instance only ever shows one ticket.
  const ticketRef = useRef<Ticket | null>(null)
  const busy = pending > 0

  /** Show `next` unless it is another ticket's or older than what is shown; true when it was shown. */
  const show = useCallback((next: Ticket): boolean => {
    if (isStale(ticketRef.current, next)) return false
    ticketRef.current = next
    setTicket(next)
    return true
  }, [])

  const load = useCallback(async () => {
    const request = requestRef.current + 1
    requestRef.current = request
    setLoading(true)
    setError(null)
    try {
      const result = await api.ticket(host.iii, ticketId)
      if (requestRef.current !== request) return
      show(result)
    } catch (caught) {
      if (requestRef.current !== request) return
      setError(errorMessage(caught))
    } finally {
      if (requestRef.current === request) setLoading(false)
    }
  }, [host, ticketId, show])

  useEffect(() => {
    void load()
  }, [load])

  // The board: label suggestions, the project, release and parent choices, the done column. Read once, and
  // again whenever one of those lists opens.
  const boardRequest = useRef(0)
  const refreshBoard = useCallback(() => {
    const request = ++boardRequest.current
    void api
      .board(host.iii)
      .then((next) => {
        if (boardRequest.current === request) setBoard(next)
      })
      .catch(() => undefined)
  }, [host])
  useEffect(() => {
    refreshBoard()
    return () => {
      boardRequest.current++
    }
  }, [refreshBoard])
  const boardTickets = useMemo(() => (board ? board.columns.flatMap((column) => column.tickets) : null), [board])
  const boardLabels = useMemo(() => allLabels(boardTickets ?? []), [boardTickets])
  const doneStatus = board?.done_status || knownDoneStatus || columns[columns.length - 1]?.id || ''
  const canceledStatus = (board ? board.canceled_status : knownCanceledStatus) ?? null

  // A subtask shows its parent's numbers and its siblings: the parent as the worker returns it.
  const parentId = ticket?.parent_id ?? null
  const parentRequest = useRef(0)
  const parentTimer = useRef<number | undefined>(undefined)
  const selfTimer = useRef<number | undefined>(undefined)
  const selfRequest = useRef(0)
  const fetchParent = useCallback(() => {
    const id = ticketRef.current?.parent_id
    const request = ++parentRequest.current
    if (!id) {
      setParentView(null)
      return
    }
    void api
      .ticket(host.iii, id)
      .then((view) => {
        if (parentRequest.current === request) setParentView(view)
      })
      .catch(() => undefined)
  }, [host])
  useEffect(() => {
    fetchParent()
    return () => {
      parentRequest.current++
    }
  }, [fetchParent, parentId])
  // A burst of events (closing ten subtasks) is one read.
  const refreshParent = useCallback(() => {
    window.clearTimeout(parentTimer.current)
    parentTimer.current = window.setTimeout(fetchParent, 300)
  }, [fetchParent])
  const refreshSelf = useCallback(() => {
    window.clearTimeout(selfTimer.current)
    selfTimer.current = window.setTimeout(() => {
      const id = ticketRef.current?.id
      if (id) {
        // Only the newest read counts: with a burst of closes the older reply can land last, and an equal
        // `updated_at` would not tell it apart.
        const request = ++selfRequest.current
        void api
          .ticket(host.iii, id)
          .then((next) => {
            if (selfRequest.current === request) show(next)
          })
          .catch(() => undefined)
      }
    }, 300)
  }, [host, show])
  useEffect(
    () => () => {
      window.clearTimeout(parentTimer.current)
      window.clearTimeout(selfTimer.current)
      selfRequest.current++
    },
    [],
  )

  // A project or release written in another pane fires no change event: the options, and the project or
  // release this ticket sits in (a shipped release moves its open tickets), are read again.
  useEffect(() => {
    const onPlanChanged = () => {
      refreshBoard()
      refreshSelf()
    }
    window.addEventListener(PLAN_CHANGED_EVENT, onPlanChanged)
    return () => window.removeEventListener(PLAN_CHANGED_EVENT, onPlanChanged)
  }, [refreshBoard, refreshSelf])

  const ticketUpdatedAt = ticket?.updated_at
  const linksRequest = useRef(0)
  const linksTimer = useRef<number | undefined>(undefined)
  const linksRef = useRef(links)
  linksRef.current = links
  // Stable on purpose: the change subscription below must not be torn down and re-registered per ticket.
  const fetchLinks = useCallback(() => {
    const key = ticketRef.current?.key
    if (!key) return
    const request = ++linksRequest.current
    void api
      .links(host.iii, key)
      .then((result) => {
        if (linksRequest.current === request) setLinks(mergeLinks(result))
      })
      // An older worker has no kanban::ticket::links: the section stays hidden.
      .catch(() => undefined)
  }, [host])
  const refetchLinks = useCallback(() => {
    window.clearTimeout(linksTimer.current)
    linksTimer.current = window.setTimeout(fetchLinks, 500)
  }, [fetchLinks])

  useEffect(() => {
    fetchLinks()
    return () => {
      linksRequest.current++
    }
  }, [fetchLinks, ticketUpdatedAt])
  useEffect(() => () => window.clearTimeout(linksTimer.current), [])

  useEffect(() => {
    return subscribeChanges(host.iii, (event) => {
      const incoming = event?.ticket
      if (!incoming) return
      const wanted = ticketId.toLowerCase()
      if (
        incoming.id === ticketRef.current?.id ||
        incoming.id === ticketId ||
        incoming.key.toLowerCase() === wanted
      ) {
        show(incoming)
        // Its status counts in the parent's progress.
        if (incoming.parent_id) refreshParent()
        return
      }
      const own = ticketRef.current
      if (!own) return
      // One of its subtasks changed: the list and the progress are the worker's to count.
      if (incoming.parent_id === own.id) refreshSelf()
      // Its parent or a sibling changed: the strip and the previous/next buttons.
      if (own.parent_id && (incoming.id === own.parent_id || incoming.parent_id === own.parent_id)) refreshParent()
      // Someone else's ticket: its row, or the text that cites this one, may have changed the links.
      if (linksRef.current.some((row) => row.ticket.id === incoming.id) || mentionsKey(incoming, own.key)) {
        refetchLinks()
      }
    })
  }, [host, ticketId, show, refetchLinks, refreshParent, refreshSelf])

  // After a ticket opens another in this pane, the control that was clicked is gone: focus the new title.
  const titleRef = useRef<HTMLHeadingElement>(null)
  const shownId = ticket?.id
  useEffect(() => {
    if (!shownId || Date.now() > focusTitleBefore) return
    focusTitleBefore = 0
    titleRef.current?.focus()
  }, [shownId])

  useEffect(() => {
    if (!setDirty) return
    const descriptionDirty = editingDescription && draft !== (ticket?.description ?? '')
    const titleDirty = editingTitle && titleDraft.trim() !== (ticket?.title ?? '')
    setDirty(descriptionDirty || titleDirty)
  }, [editingDescription, draft, ticket?.description, editingTitle, titleDraft, ticket?.title, setDirty])

  const composerId = useId()
  const linksTitleId = useId()
  const addLabelId = useId()
  const parentNoteId = useId()
  const labelsRef = useRef<HTMLDivElement>(null)
  const startEditing = useCallback(() => {
    if (!ticket || ticket.deleted_at) return
    setDraft(ticket.description)
    setEditingDescription(true)
  }, [ticket])

  const startEditingTitle = useCallback(() => {
    if (!ticket || ticket.deleted_at) return
    setTitleDraft(ticket.title)
    setEditingTitle(true)
  }, [ticket])

  useEffect(
    () =>
      commands?.register([
        { id: 'refresh', title: 'Refresh ticket', run: () => void load() },
        {
          id: 'edit-title',
          title: 'Edit title',
          enabled: () => Boolean(ticket) && !ticket?.deleted_at && !editingTitle,
          run: startEditingTitle,
        },
        {
          id: 'edit-description',
          title: 'Edit description',
          enabled: () => Boolean(ticket) && !ticket?.deleted_at && !editingDescription,
          run: startEditing,
        },
        {
          id: 'comment',
          title: 'Write a comment',
          enabled: () => Boolean(ticket) && !ticket?.deleted_at,
          run: () => document.getElementById(composerId)?.focus(),
        },
        ...(onBack ? [{ id: 'back', title: 'Back to the board', run: onBack }] : []),
      ]),
    [commands, composerId, editingDescription, editingTitle, load, onBack, startEditing, startEditingTitle, ticket],
  )

  const patch = useCallback(
    async (changes: Record<string, unknown>): Promise<boolean> => {
      const current = ticketRef.current
      if (!current) return false
      setPending((count) => count + 1)
      setActionError(null)
      try {
        const updated = await api.update(host.iii, { id: current.key, ...changes, actor: 'user' })
        noteOwnWrite(updated)
        show(updated)
        return true
      } catch (caught) {
        setActionError(errorMessage(caught))
        void load()
        return false
      } finally {
        setPending((count) => count - 1)
      }
    },
    [host, load, show],
  )

  /**
   * A write on other tickets (subtasks): runs it, shows what failed, and re-reads this ticket either way, so a
   * bulk action that stopped halfway shows how far it got. True when it worked.
   */
  const act = useCallback<Act>(
    async (work) => {
      setPending((count) => count + 1)
      setActionError(null)
      let ok = false
      try {
        await work()
        ok = true
      } catch (caught) {
        setActionError(errorMessage(caught))
      }
      try {
        const fresh = await api.ticket(host.iii, ticketRef.current?.id ?? ticketId)
        if (ok) noteOwnWrite(fresh)
        show(fresh)
      } catch {
        // The change event brings the same view.
      }
      setPending((count) => count - 1)
      return ok
    },
    [host, ticketId, show],
  )

  const { confirm, dialog: confirmDialog } = useConfirm()
  const detachFromParent = async () => {
    const parent = ticketRef.current?.parent
    if (!parent) return
    const yes = await confirm({
      title: `Detach from ${parent.key}?`,
      description: 'It stays on the board as a ticket of its own, with its own project and release.',
      details: [`${ticketRef.current?.key} — ${ticketRef.current?.title}`],
      confirmLabel: 'Detach',
    })
    if (yes) void patch({ parent: null })
  }

  const saveDescription = async () => {
    if (await patch({ description: draft })) setEditingDescription(false)
  }

  const saveTitle = async () => {
    const next = titleDraft.trim()
    if (!ticket || !next) return
    if (next === ticket.title || (await patch({ title: next }))) setEditingTitle(false)
  }

  const submitComment = async (body: string): Promise<boolean> => {
    const current = ticketRef.current
    if (!current) return false
    setPending((count) => count + 1)
    setActionError(null)
    try {
      const result = await api.comment(host.iii, {
        ticket_id: current.key,
        body,
        parent_id: replyTo?.id ?? null,
        author: 'user',
      })
      noteOwnWrite(result.ticket)
      show(result.ticket)
      setReplyTo(null)
      return true
    } catch (caught) {
      setActionError(errorMessage(caught))
      return false
    } finally {
      setPending((count) => count - 1)
    }
  }

  const toggleDeleted = async () => {
    const current = ticketRef.current
    if (!current) return
    setPending((count) => count + 1)
    setActionError(null)
    try {
      const updated = current.deleted_at
        ? await api.restore(host.iii, { id: current.key, actor: 'user' })
        : await api.remove(host.iii, { id: current.key, actor: 'user' })
      noteOwnWrite(updated)
      show(updated)
      onDeleted?.(Boolean(updated.deleted_at))
      setConfirming(false)
    } catch (caught) {
      setActionError(errorMessage(caught))
    } finally {
      setPending((count) => count - 1)
    }
  }

  /** The chip that had focus is gone or the list changed: keep focus on the add-label control unless the user moved on. */
  const refocusAddLabel = () => {
    const active = document.activeElement
    if (active && active !== document.body && !labelsRef.current?.contains(active)) return
    document.getElementById(addLabelId)?.focus()
  }

  // add_labels / remove_labels edit the labels the server has when the write runs: concurrent edits do not overwrite each other.
  const addLabel = (raw: string) => {
    const label = raw.trim()
    if (label) void patch({ add_labels: [label] }).then(refocusAddLabel)
  }

  const removeLabel = (label: string) => {
    void patch({ remove_labels: [label] }).then(refocusAddLabel)
  }

  const hasPanels = typeof host.panels?.open === 'function'
  const canOpenTicket = hasPanels || Boolean(onOpenTicket)
  const openTicket = useCallback(
    (key: string) => {
      focusTitleBefore = Date.now() + 3000
      if (hasPanels) host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id: key } })
      else onOpenTicket?.(key)
    },
    [hasPanels, host, onOpenTicket],
  )

  const linkedTickets = useMemo<LinkedTicket[]>(
    () => links.map(({ ticket: linked }) => ({ key: linked.key, title: linked.title, status: statusLabel(columns, linked.status) })),
    [links, columns],
  )

  /**
   * Clicks on `#kanban-ticket/KEY` anchors open that ticket instead of navigating: plain, modified and middle
   * (a new console tab). Only keys the screen has linked open: ticket text can carry any href under any label.
   */
  const onProseClick = useCallback(
    (event: MouseEvent<HTMLElement>) => {
      if (event.type === 'auxclick' && event.button !== 1) return
      const anchor = (event.target as Element).closest?.(`a[href^="${TICKET_HREF}"]`)
      const href = anchor?.getAttribute('href')
      if (!href) return
      event.preventDefault()
      const key = href.slice(TICKET_HREF.length)
      if (linkedTickets.some((linked) => linked.key === key)) openTicket(key)
    },
    [linkedTickets, openTicket],
  )

  const timeline = useMemo(() => {
    if (!ticket) return []
    const comments = new Map(ticket.comments.map((comment) => [comment.id, comment]))
    return [...ticket.activity]
      .sort((a, b) => a.seq - b.seq)
      .map((entry) => ({ entry, comment: entry.comment_id ? comments.get(entry.comment_id) ?? null : null }))
  }, [ticket])

  const agentOptions = useMemo(
    () =>
      agents.map((agent) => ({
        value: agent.id,
        label: agent.name,
        description: agent.description ?? undefined,
      })),
    [agents],
  )

  const priorityOptions = useMemo(() => {
    const values = priorities.length > 0 ? [...priorities] : ['low', 'medium', 'high', 'urgent']
    if (ticket && !values.includes(ticket.priority)) values.push(ticket.priority)
    return values.map((value) => ({ value, label: priorityLabel(value) }))
  }, [priorities, ticket])

  const labelOptions = useMemo(() => {
    const own = new Set(ticket?.labels.map(asciiLower))
    return boardLabels.filter((label) => !own.has(asciiLower(label))).map((label) => ({ value: label, label }))
  }, [boardLabels, ticket?.labels])

  if (loading && !ticket) {
    return (
      <PageShell {...shell}>
        <PageHeader icon={<TicketIcon size={16} />} onClose={onRequestClose} title="Ticket" />
        <PageMain>
          <div className="kanban-record">
            <div className="kanban-record__grid">
              <div className="kanban-record__head">
                <Skeleton className="kanban-skeleton kanban-skeleton--title" />
              </div>
              <div className="kanban-record__body">
                <Skeleton className="kanban-skeleton kanban-skeleton--card" />
              </div>
            </div>
          </div>
        </PageMain>
      </PageShell>
    )
  }

  if (error || !ticket) {
    return (
      <PageShell {...shell}>
        <PageHeader
          actions={
            onBack ? (
              <IconButton className="kanban-touch" label="Back to the board" onClick={onBack} variant="ghost">
                <ArrowLeft size={16} />
              </IconButton>
            ) : null
          }
          icon={<TicketIcon size={16} />}
          onClose={onRequestClose}
          title="Ticket"
        />
        <PageMain>
          <div className="kanban-pad">
            <StatusPanel
              detail={error ?? `No ticket matches ${ticketId}.`}
              headline="That ticket could not be opened"
              icon={<TriangleAlert size={16} />}
              variant="alert"
              action={
                <Button className="kanban-touch" onClick={() => void load()} variant="pill">
                  Retry
                </Button>
              }
            />
          </div>
        </PageMain>
      </PageShell>
    )
  }

  const deleted = Boolean(ticket.deleted_at)
  const agentName = (id: string) => agents.find((agent) => agent.id === id)?.name ?? id
  const commentTotal = timeline.filter(({ comment }) => comment).length
  const visibleTimeline = timeline.filter(
    ({ comment }) => activityFilter === 'all' || (activityFilter === 'comments') === Boolean(comment),
  )
  const keyRef: KeyRef = (key) => <TicketRef onOpen={canOpenTicket ? openTicket : undefined} ticketKey={key} />
  // The parent and the subtasks have their own places on this screen: they are not repeated as links.
  const related = new Set([ticket.parent_id, ...ticket.children.map((child) => child.id)])
  const shownLinks = links.filter((row) => !related.has(row.ticket.id))
  const parentLinked = links.some((row) => row.ticket.id === ticket.parent_id)
  const childLinked = links.some((row) => ticket.children.some((child) => child.id === row.ticket.id))
  const projectName = board?.projects.find((project) => project.id === ticket.project_id)?.name
  const releaseName = board?.releases.find((release) => release.id === ticket.release_id)?.name
  const parentBlocked = parentBlockedReason(ticket)
  // The parent as the worker last returned it: the copy inside this ticket goes stale when the parent is deleted elsewhere.
  const parentBrief = parentView?.id === ticket.parent_id ? briefOf(parentView) : ticket.parent
  /** "Its parent is in Phase 2 · Add to Phase 2": the parent's project or release, offered once, never applied. */
  const facetOffer = (kind: 'project' | 'release') => {
    const offer = deleted ? null : parentOffer(ticket, kind, board?.projects ?? [], board?.releases ?? [])
    if (!offer) return null
    return (
      <div className="kanban-prop__offer">
        <span>Its parent is in {offer.name}</span>
        <Button
          className="kanban-touch"
          disabled={busy}
          onClick={() => void patch({ [kind]: offer.id })}
          size="sm"
          variant="pill"
        >
          Add to {offer.name}
        </Button>
      </div>
    )
  }

  return (
    <PageShell {...shell}>
      <PageHeader
        actions={
          <>
            {onBack ? (
              <IconButton className="kanban-touch" label="Back to the board" onClick={onBack} variant="ghost">
                <ArrowLeft size={16} />
              </IconButton>
            ) : null}
            {onBack && host.panels?.open ? (
              <IconButton
                className="kanban-touch"
                label="Open in its own pane"
                onClick={() => {
                  host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id: ticket.key } })
                  onBack()
                }}
                variant="ghost"
              >
                <Columns2 size={16} />
              </IconButton>
            ) : null}
            {host.chat?.compose ? (
              <IconButton
                className="kanban-touch"
                label="Reference in chat"
                onClick={() => host.chat?.compose?.({ text: `@kanban(id="${ticket.key}") ` })}
                variant="ghost"
              >
                <AtSign size={16} />
              </IconButton>
            ) : null}
            <IconButton className="kanban-touch" label="Refresh ticket" onClick={() => void load()} variant="ghost">
              <RefreshCw size={16} />
            </IconButton>
            <IconButton
              className="kanban-touch kanban-danger"
              label={deleted ? 'Restore ticket' : 'Delete ticket'}
              onClick={() => setConfirming(true)}
              variant="ghost"
            >
              <Trash2 size={16} />
            </IconButton>
          </>
        }
        description={<span className="kanban-header-key">{ticket.key}</span>}
        icon={<TicketIcon size={16} />}
        onClose={onRequestClose}
        title="Ticket"
      />
      <PageMain>
        <div className="kanban-record" data-autofocus="" tabIndex={-1}>
          {parentBrief ? (
            <div className="kanban-record__strip">
              <ParentStrip
                canOpen={canOpenTicket}
                canceledStatus={canceledStatus}
                columns={columns}
                doneStatus={doneStatus}
                onOpen={openTicket}
                parent={parentBrief}
                parentView={parentView?.id === ticket.parent_id ? parentView : null}
                ticketId={ticket.id}
              />
            </div>
          ) : null}
          <div className="kanban-record__grid">
            <div className="kanban-record__head" data-area="head">
              <div className="kanban-record__eyebrow">
                <span className="kanban-mono">{ticket.key}</span>
                <Badge className="kanban-status-badge" variant={deleted ? 'alert' : 'default'}>
                  {deleted ? (
                    <StatusDot tone="alert" />
                  ) : (
                    <StatusGlyph
                      canceledStatus={canceledStatus}
                      columns={columns}
                      doneStatus={doneStatus}
                      status={ticket.status}
                    />
                  )}
                  {deleted ? 'Deleted' : statusLabel(columns, ticket.status)}
                </Badge>
                <PriorityChip priorities={priorities} priority={ticket.priority} />
              </div>
              {editingTitle ? (
                <div className="kanban-record__title-edit">
                  <Input
                    aria-label="Ticket title"
                    autoFocus
                    className="kanban-record__title-input"
                    onChange={setTitleDraft}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault()
                        void saveTitle()
                      } else if (event.key === 'Escape') {
                        event.preventDefault()
                        setEditingTitle(false)
                      }
                    }}
                    value={titleDraft}
                  />
                  <Button className="kanban-touch" onClick={() => setEditingTitle(false)} size="sm" variant="ghost">
                    Cancel
                  </Button>
                  <Button
                    className="kanban-touch"
                    disabled={busy || titleDraft.trim().length === 0}
                    onClick={() => void saveTitle()}
                    size="sm"
                    variant="primary"
                  >
                    Save
                  </Button>
                </div>
              ) : (
                <div className="kanban-record__titlebar">
                  <h1 className="kanban-record__title" ref={titleRef} tabIndex={-1}>
                    {ticket.title}
                  </h1>
                  {deleted ? null : (
                    <IconButton className="kanban-touch" label="Edit title" onClick={startEditingTitle} variant="ghost">
                      <Pencil size={16} />
                    </IconButton>
                  )}
                </div>
              )}
              <div className="kanban-record__meta">
                <Clock size={16} />
                <span>
                  Updated{' '}
                  <time dateTime={ticket.updated_at} title={absoluteTime(ticket.updated_at)}>
                    {ago(ticket.updated_at)}
                  </time>
                </span>
                <span className="kanban-dot" />
                <span>
                  Created{' '}
                  <time dateTime={ticket.created_at} title={absoluteTime(ticket.created_at)}>
                    {ago(ticket.created_at)}
                  </time>{' '}
                  by {agentName(ticket.created_by)}
                </span>
              </div>
              <DoneWarning
                busy={busy}
                canceledStatus={canceledStatus}
                columns={columns}
                doneStatus={doneStatus}
                host={host}
                onAct={act}
                ticket={ticket}
              />
            </div>

            <aside aria-label="Properties" className="kanban-record__props" data-area="props">
              <Panel>
                <PanelHeader>Properties</PanelHeader>
                <PanelBody className="kanban-props">
                  <div className="kanban-prop">
                    <span className="kanban-prop__label">Status</span>
                    <Select
                      appearance={narrow ? 'default' : 'inline'}
                      aria-label="Status"
                      className="kanban-touch"
                      disabled={deleted}
                      onChange={(next) => void patch({ status: next })}
                      options={columns.map((column) => ({ value: column.id, label: column.label }))}
                      value={ticket.status}
                    />
                  </div>
                  <div className="kanban-prop">
                    <span className="kanban-prop__label">Priority</span>
                    <Select
                      appearance={narrow ? 'default' : 'inline'}
                      aria-label="Priority"
                      className="kanban-touch"
                      disabled={deleted}
                      onChange={(next) => void patch({ priority: next })}
                      options={priorityOptions}
                      value={ticket.priority}
                    />
                  </div>
                  <div className="kanban-prop">
                    <span className="kanban-prop__label">Assignee</span>
                    <Selector
                      allowEmpty
                      aria-label="Assignee"
                      className="kanban-selector"
                      disabled={deleted}
                      emptyLabel="Unassigned"
                      onChange={(next) => void patch({ assignee: next })}
                      onClear={() => void patch({ assignee: null })}
                      options={agentOptions}
                      placeholder="Unassigned"
                      value={ticket.assignee ?? undefined}
                    />
                  </div>
                  <div className="kanban-prop kanban-prop--facet">
                    <span className="kanban-prop__label">Project</span>
                    <div className="kanban-prop__stack">
                      <div className="kanban-prop__row">
                        <ProjectSelector
                          disabled={deleted}
                          onChange={(next) => void patch({ project: next })}
                          onOpen={refreshBoard}
                          projects={board?.projects ?? null}
                          value={ticket.project_id}
                        />
                        {ticket.project_id && hasPanels ? (
                          <IconButton
                            className="kanban-touch"
                            label={projectName ? `Open project ${projectName}` : 'Open project'}
                            onClick={() =>
                              host.panels?.open({ pageId: PROJECTS_PAGE_ID, context: { id: ticket.project_id } })
                            }
                            variant="ghost"
                          >
                            <ArrowUpRight size={16} />
                          </IconButton>
                        ) : null}
                      </div>
                      {facetOffer('project')}
                    </div>
                  </div>
                  <div className="kanban-prop kanban-prop--facet">
                    <span className="kanban-prop__label">Release</span>
                    <div className="kanban-prop__stack">
                      <div className="kanban-prop__row">
                        <ReleaseSelector
                          disabled={deleted}
                          onChange={(next) => void patch({ release: next })}
                          onOpen={refreshBoard}
                          releases={board?.releases ?? null}
                          value={ticket.release_id}
                        />
                        {ticket.release_id && hasPanels ? (
                          <IconButton
                            className="kanban-touch"
                            label={releaseName ? `Open release ${releaseName}` : 'Open release'}
                            onClick={() =>
                              host.panels?.open({ pageId: RELEASES_PAGE_ID, context: { id: ticket.release_id } })
                            }
                            variant="ghost"
                          >
                            <ArrowUpRight size={16} />
                          </IconButton>
                        ) : null}
                      </div>
                      {facetOffer('release')}
                    </div>
                  </div>
                  <div className="kanban-prop kanban-prop--facet">
                    <span className="kanban-prop__label">Parent</span>
                    <div className="kanban-prop__stack">
                      <div className="kanban-prop__row">
                        <ParentSelector
                          aria-describedby={parentBlocked ? parentNoteId : undefined}
                          current={parentBrief}
                          disabled={deleted || parentBlocked !== null}
                          onChange={(next) => void patch({ parent: next })}
                          onOpen={refreshBoard}
                          selfId={ticket.id}
                          tickets={boardTickets}
                          value={ticket.parent?.key ?? null}
                        />
                        {ticket.parent && canOpenTicket ? (
                          <IconButton
                            className="kanban-touch"
                            label={`Open parent ${ticket.parent.key}`}
                            onClick={() => openTicket(ticket.parent?.key ?? '')}
                            variant="ghost"
                          >
                            <ArrowUpRight size={16} />
                          </IconButton>
                        ) : null}
                      </div>
                      {parentBlocked ? (
                        <p className="kanban-prop__note" id={parentNoteId}>
                          {parentBlocked}
                        </p>
                      ) : null}
                      {ticket.parent && !deleted ? (
                        <div>
                          <Button
                            className="kanban-touch"
                            disabled={busy}
                            onClick={() => void detachFromParent()}
                            size="sm"
                            variant="ghost"
                          >
                            Detach…
                          </Button>
                        </div>
                      ) : null}
                    </div>
                  </div>
                  <div className="kanban-prop kanban-prop--labels" data-empty={ticket.labels.length === 0 ? 'true' : undefined}>
                    <span className="kanban-prop__label">Labels</span>
                    <div className="kanban-prop__stack" ref={labelsRef}>
                      {ticket.labels.length > 0 ? (
                        <span className="kanban-prop__chips">
                          {[...new Set(ticket.labels)].map((label) => (
                            <Chip key={label}>
                              {label}
                              <IconButton
                                className="kanban-chip-x"
                                disabled={deleted}
                                label={`Remove label ${label}`}
                                onClick={() => removeLabel(label)}
                                tooltip={false}
                                variant="ghost"
                              >
                                <X size={16} />
                              </IconButton>
                            </Chip>
                          ))}
                        </span>
                      ) : null}
                      <Selector
                        aria-label="Add label"
                        className="kanban-selector"
                        createOptionLabel={(query) => `Create label "${query}"`}
                        disabled={deleted}
                        id={addLabelId}
                        onChange={addLabel}
                        onCreate={addLabel}
                        options={labelOptions}
                        placeholder="Add label"
                        searchPlaceholder="Find or create a label…"
                        triggerIcon={<Plus size={16} />}
                        value={undefined}
                      />
                    </div>
                  </div>
                  {ticket.pills?.length ? (
                    <div className="kanban-prop kanban-prop--static">
                      <span className="kanban-prop__label">Pills</span>
                      <Pills pills={ticket.pills} />
                    </div>
                  ) : null}
                  <div className="kanban-prop kanban-prop--static">
                    <span className="kanban-prop__label">Created</span>
                    <span className="kanban-prop__value">
                      <time dateTime={ticket.created_at} title={absoluteTime(ticket.created_at)}>
                        {ago(ticket.created_at)}
                      </time>{' '}
                      <span className="kanban-prop__by">by {agentName(ticket.created_by)}</span>
                    </span>
                  </div>
                  <div className="kanban-prop kanban-prop--static">
                    <span className="kanban-prop__label">Comments</span>
                    <span className="kanban-prop__value">{ticket.comments.length}</span>
                  </div>
                  <div className="kanban-prop kanban-prop--static">
                    <span className="kanban-prop__label">Ticket id</span>
                    <span className="kanban-prop__copy">
                      <span className="kanban-mono kanban-prop__value">{ticket.id}</span>
                      <CopyId text={ticket.id} />
                    </span>
                  </div>
                </PanelBody>
              </Panel>
            </aside>

            <section className="kanban-record__body" data-area="body">
              {deleted ? (
                <StatusPanel
                  className="kanban-record__banner"
                  detail="It is hidden from the board but the row is still in the board file."
                  headline="This ticket was deleted"
                  icon={<TriangleAlert size={16} />}
                  variant="warn"
                />
              ) : null}
              {actionError ? (
                <StatusPanel
                  action={
                    <Button className="kanban-touch" onClick={() => setActionError(null)} variant="pill">
                      Dismiss
                    </Button>
                  }
                  className="kanban-record__banner"
                  detail={actionError}
                  headline="That change was not saved"
                  icon={<TriangleAlert size={16} />}
                  variant="alert"
                />
              ) : null}

              <div className="kanban-section__head">
                <h2 className="kanban-section__title">Description</h2>
                {editingDescription ? (
                  <div className="kanban-section__actions">
                    <Button
                      className="kanban-touch"
                      onClick={() => {
                        setEditingDescription(false)
                        setDraft(ticket.description)
                      }}
                      size="sm"
                      variant="ghost"
                    >
                      Cancel
                    </Button>
                    <Button
                      className="kanban-touch"
                      disabled={busy}
                      onClick={() => void saveDescription()}
                      size="sm"
                      variant="primary"
                    >
                      Save
                    </Button>
                  </div>
                ) : (
                  <div className="kanban-section__actions">
                    <Button
                      className="kanban-touch"
                      disabled={deleted}
                      onClick={startEditing}
                      size="sm"
                      variant="ghost"
                    >
                      Edit
                    </Button>
                  </div>
                )}
              </div>
              {editingDescription ? (
                <div
                  className="kanban-editor"
                  onKeyDown={(event) => {
                    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
                      event.preventDefault()
                      void saveDescription()
                    }
                  }}
                >
                  <CodeEditor
                    aria-label="Ticket description"
                    language="markdown"
                    onChange={setDraft}
                    value={draft}
                    wordWrap
                  />
                </div>
              ) : ticket.description.trim() ? (
                <Prose onOpen={onProseClick} text={ticket.description} tickets={linkedTickets} />
              ) : (
                <p className="kanban-muted">No description yet.</p>
              )}
              {ticket.parent_id ? null : (
                <SubtasksSection
                  agents={agents}
                  busy={busy}
                  canOpen={canOpenTicket}
                  canceledStatus={canceledStatus}
                  columns={columns}
                  deleted={deleted}
                  doneStatus={doneStatus}
                  host={host}
                  onAct={act}
                  onOpen={openTicket}
                  onPick={refreshBoard}
                  priorities={priorities}
                  projects={board?.projects ?? []}
                  releases={board?.releases ?? []}
                  ticket={ticket}
                  tickets={boardTickets}
                />
              )}
              {shownLinks.length > 0 ? (
                <section aria-labelledby={linksTitleId} className="kanban-links">
                  <div className="kanban-section__head">
                    <h2 className="kanban-section__title" id={linksTitleId}>
                      Linked tickets
                    </h2>
                    <span className="kanban-section__count kanban-section__count--inline">{shownLinks.length}</span>
                  </div>
                  <p className="kanban-muted">
                    Tickets this one mentions, and tickets that mention it.
                    {parentLinked ? ` Its parent, ${ticket.parent?.key}, is shown above.` : ''}
                    {childLinked ? ' Its subtasks are listed above.' : ''}
                  </p>
                  <ul className="kanban-links__list">
                    {shownLinks.map(({ ticket: linked, relation }) => (
                      <li key={linked.id}>
                        <button
                          className="kanban-link-row"
                          disabled={!canOpenTicket}
                          onClick={() => openTicket(linked.key)}
                          type="button"
                        >
                          <StatusGlyph
                            canceledStatus={canceledStatus}
                            columns={columns}
                            doneStatus={doneStatus}
                            status={linked.status}
                          />
                          <span className="kanban-mono">{linked.key}</span>
                          <span className="kanban-link-row__title">{linked.title}</span>
                          <span className="kanban-link-row__meta">
                            {statusLabel(columns, linked.status)} · {relation}
                          </span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </section>
              ) : null}
            </section>

            <section className="kanban-record__activity" data-area="activity">
              <div className="kanban-section__head">
                <h2 className="kanban-section__title">Activity</h2>
                <span className="kanban-section__count kanban-section__count--inline">{timeline.length} entries</span>
                <SegmentedControl
                  aria-label="Show in activity"
                  className="kanban-activity-filter"
                  itemClassName="kanban-touch"
                  onChange={setActivityFilter}
                  options={[
                    { value: 'all', label: 'All' },
                    { value: 'comments', label: `Comments ${commentTotal}` },
                    { value: 'changes', label: `Changes ${timeline.length - commentTotal}` },
                  ]}
                  value={activityFilter}
                  variant="radio"
                />
              </div>
              {timeline.length === 0 ? (
                <p className="kanban-muted">Nothing has happened yet.</p>
              ) : visibleTimeline.length === 0 ? (
                <p className="kanban-muted">No {activityFilter} yet.</p>
              ) : (
                <ol className="kanban-timeline">
                  {groupSubtaskAdds(visibleTimeline).map((row) => {
                    if (row.kind === 'adds') {
                      const first = row.items[0]?.entry
                      const last = row.items[row.items.length - 1]?.entry
                      if (!first || !last) return null
                      return (
                        <li className="kanban-event" key={first.id}>
                          <span className="kanban-event__icon">
                            <ListTree size={16} />
                          </span>
                          <span className="kanban-event__text">
                            <strong>{agentName(first.actor)}</strong> added {row.keys.length} subtasks{' '}
                            <span title={row.keys.join(', ')}>
                              {keyRef(row.keys[0] ?? '')} … {keyRef(row.keys[row.keys.length - 1] ?? '')}
                            </span>
                          </span>
                          <time
                            className="kanban-mono kanban-event__time"
                            dateTime={last.at}
                            title={absoluteTime(last.at)}
                          >
                            {ago(last.at)}
                          </time>
                        </li>
                      )
                    }
                    const { entry, comment } = row.item
                    if (comment) {
                      const author = agents.find((agent) => agent.id === comment.author)
                      return (
                        <li
                          className="kanban-comment"
                          data-reply={comment.parent_id ? 'true' : undefined}
                          key={entry.id}
                        >
                          <div className="kanban-comment__head">
                            {author ? (
                              <span
                                aria-hidden="true"
                                className={`${uiClasses.treeItemIcon} kanban-comment__agent`}
                                data-color={author.color ?? undefined}
                              >
                                <Bot size={16} />
                              </span>
                            ) : (
                              <span aria-hidden="true" className="kanban-avatar">
                                {comment.author.slice(0, 1).toUpperCase()}
                              </span>
                            )}
                            <span className="kanban-comment__author">{agentName(comment.author)}</span>
                            <time
                              className="kanban-mono kanban-comment__time"
                              dateTime={comment.created_at}
                              title={absoluteTime(comment.created_at)}
                            >
                              {ago(comment.created_at)}
                            </time>
                            <Button
                              className="kanban-touch kanban-comment__reply"
                              disabled={deleted}
                              onClick={() => setReplyTo({ id: comment.id, author: comment.author })}
                              size="sm"
                              variant="ghost"
                            >
                              Reply
                            </Button>
                          </div>
                          <Prose onOpen={onProseClick} text={comment.body} tickets={linkedTickets} />
                        </li>
                      )
                    }
                    const EventIcon = activityIcon(entry)
                    return (
                      <li className="kanban-event" key={entry.id}>
                        <span className="kanban-event__icon">
                          <EventIcon size={16} />
                        </span>
                        <span className="kanban-event__text">
                          <strong>{agentName(entry.actor)}</strong> {activitySentence(entry, columns, agentName, keyRef)}
                        </span>
                        <time
                          className="kanban-mono kanban-event__time"
                          dateTime={entry.at}
                          title={absoluteTime(entry.at)}
                        >
                          {ago(entry.at)}
                        </time>
                      </li>
                    )
                  })}
                </ol>
              )}
              {deleted ? null : (
                <Composer
                  busy={busy}
                  inputId={composerId}
                  onCancelReply={() => setReplyTo(null)}
                  onSubmit={submitComment}
                  replyTo={replyTo}
                />
              )}
            </section>
          </div>
        </div>
      </PageMain>
      <ConfirmDialog
        confirmLabel={deleted ? 'Restore' : 'Delete'}
        description={
          deleted
            ? 'The ticket returns to its column on the board.'
            : 'It disappears from the board but stays in the board file, and its comments are kept.'
        }
        details={[`${ticket.key} — ${ticket.title}`]}
        onCancel={() => setConfirming(false)}
        onConfirm={() => void toggleDeleted()}
        onOpenChange={setConfirming}
        open={confirming}
        title={deleted ? 'Restore this ticket?' : 'Delete this ticket?'}
      />
      {confirmDialog}
    </PageShell>
  )
}

export function TicketPage({
  host,
  onRequestClose,
  panelContext,
  paneId,
  tabId,
  setDirty,
  commands,
}: PageRenderProps & { host: Host }) {
  const key = `kanban:ticket:${paneId ?? tabId ?? 'default'}`
  const [storedId, setStoredId] = usePaneState<string | null>(key, null)
  const contextId = readContextId(panelContext?.context)
  const [columns, setColumns] = useState<Column[]>([])
  const [priorities, setPriorities] = useState<string[]>([])
  const [doneStatus, setDoneStatus] = useState('')
  const [canceledStatus, setCanceledStatus] = useState<string | null>(null)
  const [agents, setAgents] = useState<AgentProfile[]>([])
  const { props: shell } = useShell()

  useEffect(() => {
    if (contextId) setStoredId(contextId)
  }, [contextId, panelContext?.id, setStoredId])

  useEffect(() => {
    let cancelled = false
    void api
      .config(host.iii)
      .then((info) => {
        if (cancelled) return
        setColumns(info.columns ?? [])
        setPriorities(info.priorities ?? [])
        setDoneStatus(info.done_status ?? '')
        setCanceledStatus(info.canceled_status ?? null)
      })
      .catch(() => undefined)
    void api
      .agents(host.iii)
      .then((result) => {
        if (!cancelled) setAgents(result.agents ?? [])
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [host])

  const id = contextId ?? storedId

  if (!id) {
    return (
      <PageShell {...shell}>
        <PageHeader icon={<TicketIcon size={16} />} onClose={onRequestClose} title="Ticket" />
        <PageMain>
          <div className="kanban-pad">
            <EmptyState
              action={
                host.panels?.open
                  ? { label: 'Open the board', onClick: () => host.panels?.open({ pageId: BOARD_PAGE_ID }) }
                  : undefined
              }
              description="Open one from the board, or mention it in chat."
              icon={TicketIcon}
              title="No ticket selected"
            />
          </div>
        </PageMain>
      </PageShell>
    )
  }

  return (
    <TicketScreen
      agents={agents}
      columns={columns}
      canceledStatus={canceledStatus}
      commands={commands}
      doneStatus={doneStatus}
      host={host}
      // One screen per ticket: drafts, reply target and errors never travel to the next one.
      key={id}
      onRequestClose={onRequestClose}
      priorities={priorities}
      setDirty={setDirty}
      ticketId={id}
    />
  )
}

