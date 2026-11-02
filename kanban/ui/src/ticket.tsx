import {
  ConfirmDialog,
  type Host,
  Markdown,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  ShadcnBadge,
  ShadcnButton,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuItem,
  ShadcnDropdownMenuSeparator,
  ShadcnDropdownMenuTrigger,
  ShadcnSkeleton,
  ShadcnTabs,
  ShadcnTabsList,
  ShadcnTabsTrigger,
  ShadcnTextarea,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useContainerNarrow, useCopyFlash, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUpRight,
  AtSign,
  Check,
  ChevronRight,
  CirclePlus,
  Columns2,
  Copy,
  Ellipsis,
  type LucideIcon,
  Milestone,
  ListTree,
  Pencil,
  RefreshCw,
  RotateCcw,
  Tag,
  Ticket as TicketIcon,
  Trash2,
  TriangleAlert,
  X,
} from 'lucide-react'
import { Fragment, memo, type MouseEvent, type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { statusStep } from './board-state'
import { allLabels } from './filters'
import { type LinkedTicket, linkTicketKeys, TICKET_HREF } from './links'
import {
  type Activity,
  type AgentProfile,
  api,
  BOARD_PAGE_ID,
  type BoardData,
  type Brief,
  CONFIG_EVENT,
  type Change,
  type Column,
  noteOwnWrite,
  PLAN_CHANGED_EVENT,
  PROJECTS_PAGE_ID,
  Pills,
  RELEASES_PAGE_ID,
  statusLabel,
  subscribeChanges,
  TICKET_PAGE_ID,
  type Ticket,
  type TicketLinks,
  type TicketSummary,
} from './shared'
import {
  groupSubtaskAdds,
  type Part,
  parentBlockedReason,
  parentOffer,
  parentSentence,
  planSentence,
  subtaskSentence,
} from './subtask-rules'
import { type Act, DoneWarning, ParentStrip, SubtasksSection, TicketRef } from './subtasks'
import { LabelAdder } from './ticket/label-adder'
import { AssigneeAvatar, assigneeName } from './ui/avatar'
import { Empty, Notice } from './ui/feedback'
import { StatusIcon } from './ui/glyphs'
import { IconAction } from './ui/icon-action'
import { LabelBadge } from './ui/label-badge'
import { AssigneePicker, ParentPicker, PriorityMenu, ProjectPicker, ReleasePicker, StatusMenu } from './ui/properties'

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
      <span className="kb-event__labels">
        {part.labels.map((label) => (
          <LabelBadge key={label} label={label} />
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
      return only('labels') ? Tag : only('parent') ? ListTree : only('project', 'release') ? Milestone : Pencil
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

/** Props of the pane root: `data-narrow` below 720px gives the controls touch-size targets (ticket.css). */
function useShell() {
  const { ref, narrow } = useContainerNarrow()
  return { narrow, props: { className: 'kb kb-shell', 'data-narrow': narrow ? 'true' : undefined, ref } }
}

function CopyId({ text }: { text: string }) {
  const { state, copy } = useCopyFlash(text)
  return (
    <>
      <IconAction label="Copy ticket id" onClick={() => copy()} size="icon-xs">
        {state === 'copied' ? <Check size={16} /> : state === 'failed' ? <TriangleAlert size={16} /> : <Copy size={16} />}
      </IconAction>
      <span className="kb-sr-only" role="status">
        {state === 'failed' ? 'Copy failed' : state === 'copied' ? 'Copied' : ''}
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
    <div className="kb-prose" onAuxClickCapture={onOpen} onClickCapture={onOpen}>
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
    <div className="kb-composer" data-reply={replyTo ? 'true' : undefined}>
      {replyTo ? (
        <div className="kb-composer__reply">
          <span>
            Replying to <strong>{replyTo.author}</strong>
          </span>
          <ShadcnButton onClick={cancelReply} size="xs" type="button" variant="ghost">
            Cancel
          </ShadcnButton>
        </div>
      ) : null}
      <ShadcnTextarea
        aria-label={replyTo ? 'Reply to comment' : 'Add a comment'}
        className="kb-composer__input"
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
        placeholder={replyTo ? 'Write a reply…' : 'Leave a comment…'}
        rows={3}
        value={body}
      />
      <div className="kb-composer__foot">
        <span className="kb-composer__hint">Markdown · ⌘↵ to send</span>
        <ShadcnButton disabled={busy || body.trim().length === 0} onClick={() => void submit()} size="sm" type="button">
          {replyTo ? 'Reply' : 'Comment'}
        </ShadcnButton>
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
  // The page's configuration is read again on `config.updated`; the board data is not, so it comes second.
  const doneStatus = knownDoneStatus || board?.done_status || columns[columns.length - 1]?.id || ''
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
  const activityTitleId = useId()
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

  if (loading && !ticket) {
    return (
      <PageShell {...shell}>
        <PageHeader icon={<TicketIcon size={16} />} onClose={onRequestClose} title="Ticket" />
        <PageMain>
          <div aria-busy="true" aria-label="Loading the ticket" className="kb-ticket">
            <div className="kb-ticket__layout">
              <div className="kb-ticket__main">
                <ShadcnSkeleton className="kb-ticket__sk kb-ticket__sk--key" />
                <ShadcnSkeleton className="kb-ticket__sk kb-ticket__sk--title" />
                <ShadcnSkeleton className="kb-ticket__sk kb-ticket__sk--body" />
              </div>
              <div className="kb-ticket__aside">
                <ShadcnSkeleton className="kb-ticket__sk kb-ticket__sk--props" />
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
              <IconAction label="Back to the board" onClick={onBack}>
                <ArrowLeft size={16} />
              </IconAction>
            ) : null
          }
          icon={<TicketIcon size={16} />}
          onClose={onRequestClose}
          title="Ticket"
        />
        <PageMain>
          <div className="kb-ticket">
            <Notice
              actionLabel="Retry"
              detail={error ?? `No ticket matches ${ticketId}.`}
              onAction={() => void load()}
              title="That ticket could not be opened"
            />
          </div>
        </PageMain>
      </PageShell>
    )
  }

  const deleted = Boolean(ticket.deleted_at)
  const agentName = (id: string) => assigneeName(id, agents)
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
  const step = (status: string) => statusStep(columns, status, doneStatus, canceledStatus)
  const openBoard = onBack ?? (hasPanels ? () => host.panels?.open({ pageId: BOARD_PAGE_ID }) : undefined)
  /** "Its parent is in Phase 2 · Add to Phase 2": the parent's project or release, offered once, never applied. */
  const facetOffer = (kind: 'project' | 'release') => {
    const offer = deleted ? null : parentOffer(ticket, kind, board?.projects ?? [], board?.releases ?? [])
    if (!offer) return null
    return (
      <div className="kb-propnote">
        <span>Its parent is in {offer.name}</span>
        <ShadcnButton disabled={busy} onClick={() => void patch({ [kind]: offer.id })} size="xs" type="button" variant="outline">
          Add to {offer.name}
        </ShadcnButton>
      </div>
    )
  }

  return (
    <PageShell {...shell}>
      <PageHeader
        actions={
          <>
            {onBack ? (
              <IconAction label="Back to the board" onClick={onBack}>
                <ArrowLeft size={16} />
              </IconAction>
            ) : null}
            {onBack && host.panels?.open ? (
              <IconAction
                label="Open in its own pane"
                onClick={() => {
                  host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id: ticket.key } })
                  onBack()
                }}
              >
                <Columns2 size={16} />
              </IconAction>
            ) : null}
            <ShadcnDropdownMenu>
              <ShadcnDropdownMenuTrigger
                render={<ShadcnButton aria-label="Ticket actions" size="icon-sm" type="button" variant="ghost" />}
              >
                <Ellipsis size={16} />
              </ShadcnDropdownMenuTrigger>
              <ShadcnDropdownMenuContent align="end" className="kb-menu">
                <ShadcnDropdownMenuGroup>
                  {host.chat?.compose ? (
                    <ShadcnDropdownMenuItem onClick={() => host.chat?.compose?.({ text: `@kanban(id="${ticket.key}") ` })}>
                      <AtSign size={16} />
                      Reference in chat
                    </ShadcnDropdownMenuItem>
                  ) : null}
                  <ShadcnDropdownMenuItem onClick={() => void navigator.clipboard?.writeText(ticket.key)}>
                    <Copy size={16} />
                    Copy key
                  </ShadcnDropdownMenuItem>
                  <ShadcnDropdownMenuItem onClick={() => void load()}>
                    <RefreshCw size={16} />
                    Refresh
                  </ShadcnDropdownMenuItem>
                </ShadcnDropdownMenuGroup>
                <ShadcnDropdownMenuSeparator />
                <ShadcnDropdownMenuItem onClick={() => setConfirming(true)} variant={deleted ? 'default' : 'destructive'}>
                  {deleted ? <RotateCcw size={16} /> : <Trash2 size={16} />}
                  {deleted ? 'Restore ticket…' : 'Delete ticket'}
                </ShadcnDropdownMenuItem>
              </ShadcnDropdownMenuContent>
            </ShadcnDropdownMenu>
          </>
        }
        description={<span className="kb-header-key">{ticket.key}</span>}
        icon={<TicketIcon size={16} />}
        onClose={onRequestClose}
        title="Ticket"
      />
      <PageMain>
        <div className="kb-ticket" data-autofocus="" tabIndex={-1}>
          <nav aria-label="Breadcrumb" className="kb-crumbs">
            {openBoard ? (
              <button aria-label="Back to the board" className="kb-crumbs__link kb-crumbs__back" onClick={openBoard} type="button">
                <ArrowLeft size={16} />
                Kanban
              </button>
            ) : (
              <span className="kb-crumbs__item">Kanban</span>
            )}
            {parentBrief ? (
              <>
                <ChevronRight aria-hidden="true" size={16} />
                <button
                  className="kb-crumbs__link"
                  disabled={!canOpenTicket}
                  onClick={() => openTicket(parentBrief.key)}
                  type="button"
                >
                  {parentBrief.key}
                </button>
              </>
            ) : null}
            <ChevronRight aria-hidden="true" size={16} />
            <span aria-current="page" className="kb-crumbs__item">
              {ticket.key}
            </span>
            {deleted ? (
              <ShadcnBadge className="kb-crumbs__badge" variant="destructive">
                Deleted
              </ShadcnBadge>
            ) : null}
          </nav>

          <div className="kb-ticket__layout">
            <div className="kb-ticket__main">
              {parentBrief ? (
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
              ) : null}

              {deleted ? (
                <div className="kb-banner" data-tone="warn" role="status">
                  <TriangleAlert size={16} />
                  <div className="kb-banner__text">
                    <p className="kb-banner__title">This ticket was deleted</p>
                    <p className="kb-banner__detail">It is hidden from the board but the row is still in the board file.</p>
                  </div>
                </div>
              ) : null}
              {actionError ? (
                <Notice
                  actionLabel="Dismiss"
                  detail={actionError}
                  onAction={() => setActionError(null)}
                  title="That change was not saved"
                />
              ) : null}
              <DoneWarning
                busy={busy}
                canceledStatus={canceledStatus}
                columns={columns}
                doneStatus={doneStatus}
                host={host}
                onAct={act}
                ticket={ticket}
              />

              {editingTitle ? (
                <div className="kb-title-edit">
                  <input
                    aria-label="Ticket title"
                    autoFocus
                    className="kb-title-edit__input"
                    onChange={(event) => setTitleDraft(event.target.value)}
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
                  <div className="kb-title-edit__actions">
                    <ShadcnButton onClick={() => setEditingTitle(false)} size="sm" type="button" variant="ghost">
                      Cancel
                    </ShadcnButton>
                    <ShadcnButton
                      disabled={busy || titleDraft.trim().length === 0}
                      onClick={() => void saveTitle()}
                      size="sm"
                      type="button"
                    >
                      Save
                    </ShadcnButton>
                  </div>
                </div>
              ) : (
                <div className="kb-titlebar">
                  <h1 className="kb-titlebar__title" ref={titleRef} tabIndex={-1}>
                    {ticket.title}
                  </h1>
                  {deleted ? null : (
                    <span className="kb-titlebar__edit">
                      <IconAction label="Edit title" onClick={startEditingTitle} size="icon-sm">
                        <Pencil size={16} />
                      </IconAction>
                    </span>
                  )}
                </div>
              )}

              <section aria-label="Description" className="kb-description">
                {editingDescription ? (
                  <div
                    className="kb-description__edit"
                    onKeyDown={(event) => {
                      if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
                        event.preventDefault()
                        void saveDescription()
                      } else if (event.key === 'Escape') {
                        event.preventDefault()
                        setEditingDescription(false)
                        setDraft(ticket.description)
                      }
                    }}
                  >
                    <ShadcnTextarea
                      aria-label="Ticket description"
                      autoFocus
                      className="kb-description__input"
                      onChange={(event) => setDraft(event.target.value)}
                      placeholder="Add a description… (Markdown)"
                      rows={8}
                      value={draft}
                    />
                    <div className="kb-description__actions">
                      <span className="kb-composer__hint">Markdown · ⌘↵ to save · Esc to cancel</span>
                      <ShadcnButton
                        onClick={() => {
                          setEditingDescription(false)
                          setDraft(ticket.description)
                        }}
                        size="sm"
                        type="button"
                        variant="ghost"
                      >
                        Cancel
                      </ShadcnButton>
                      <ShadcnButton disabled={busy} onClick={() => void saveDescription()} size="sm" type="button">
                        Save
                      </ShadcnButton>
                    </div>
                  </div>
                ) : ticket.description.trim() ? (
                  <div className="kb-description__read">
                    <Prose onOpen={onProseClick} text={ticket.description} tickets={linkedTickets} />
                    {deleted ? null : (
                      <span className="kb-description__edit-action">
                        <IconAction label="Edit description" onClick={startEditing} size="icon-xs">
                          <Pencil size={16} />
                        </IconAction>
                      </span>
                    )}
                  </div>
                ) : (
                  <button className="kb-description__empty" disabled={deleted} onClick={startEditing} type="button">
                    Add a description…
                  </button>
                )}
              </section>

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
                <section aria-labelledby={linksTitleId} className="kb-section">
                  <header className="kb-section__head">
                    <h2 className="kb-section__title" id={linksTitleId}>
                      Linked tickets
                    </h2>
                    <span className="kb-section__count">{shownLinks.length}</span>
                  </header>
                  <p className="kb-muted">
                    Tickets this one mentions, and tickets that mention it.
                    {parentLinked ? ` Its parent, ${ticket.parent?.key}, is shown above.` : ''}
                    {childLinked ? ' Its subtasks are listed above.' : ''}
                  </p>
                  <ul className="kb-rows">
                    {shownLinks.map(({ ticket: linked, relation }) => (
                      <li className="kb-row" key={linked.id}>
                        <button
                          className="kb-row__main"
                          disabled={!canOpenTicket}
                          onClick={() => openTicket(linked.key)}
                          type="button"
                        >
                          <StatusIcon step={step(linked.status)} />
                          <span className="kb-mono kb-row__key">{linked.key}</span>
                          <span className="kb-row__title">{linked.title}</span>
                          <span className="kb-row__hint">{relation}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </section>
              ) : null}

              <section aria-labelledby={activityTitleId} className="kb-section kb-activity">
                <header className="kb-section__head">
                  <h2 className="kb-section__title" id={activityTitleId}>
                    Activity
                  </h2>
                  <span className="kb-section__spacer" />
                  <ShadcnTabs
                    className="kb-section__tabs"
                    onValueChange={(next) =>
                      setActivityFilter(next === 'comments' ? 'comments' : next === 'changes' ? 'changes' : 'all')
                    }
                    value={activityFilter}
                  >
                    <ShadcnTabsList aria-label="Show in activity">
                      <ShadcnTabsTrigger value="all">All</ShadcnTabsTrigger>
                      <ShadcnTabsTrigger value="comments">
                        Comments <span className="kb-section__tabcount">{commentTotal}</span>
                      </ShadcnTabsTrigger>
                      <ShadcnTabsTrigger value="changes">
                        Changes <span className="kb-section__tabcount">{timeline.length - commentTotal}</span>
                      </ShadcnTabsTrigger>
                    </ShadcnTabsList>
                  </ShadcnTabs>
                </header>
                {timeline.length === 0 ? (
                  <p className="kb-muted">Nothing has happened yet.</p>
                ) : visibleTimeline.length === 0 ? (
                  <p className="kb-muted">No {activityFilter} yet.</p>
                ) : (
                  <ol className="kb-timeline">
                    {groupSubtaskAdds(visibleTimeline).map((row) => {
                      if (row.kind === 'adds') {
                        const first = row.items[0]?.entry
                        const last = row.items[row.items.length - 1]?.entry
                        if (!first || !last) return null
                        return (
                          <li className="kb-event" key={first.id}>
                            <span aria-hidden="true" className="kb-event__icon">
                              <ListTree size={16} />
                            </span>
                            <span className="kb-event__text">
                              <strong>{agentName(first.actor)}</strong> added {row.keys.length} subtasks{' '}
                              <span title={row.keys.join(', ')}>
                                {keyRef(row.keys[0] ?? '')} … {keyRef(row.keys[row.keys.length - 1] ?? '')}
                              </span>
                            </span>
                            <time className="kb-event__time" dateTime={last.at} title={absoluteTime(last.at)}>
                              {ago(last.at)}
                            </time>
                          </li>
                        )
                      }
                      const { entry, comment } = row.item
                      if (comment) {
                        return (
                          <li className="kb-comment" data-reply={comment.parent_id ? 'true' : undefined} key={entry.id}>
                            <div className="kb-comment__head">
                              <AssigneeAvatar agents={agents} id={comment.author} />
                              <span className="kb-comment__author">{agentName(comment.author)}</span>
                              <time
                                className="kb-comment__time"
                                dateTime={comment.created_at}
                                title={absoluteTime(comment.created_at)}
                              >
                                {ago(comment.created_at)}
                              </time>
                              <span className="kb-section__spacer" />
                              <ShadcnButton
                                className="kb-comment__reply"
                                disabled={deleted}
                                onClick={() => setReplyTo({ id: comment.id, author: agentName(comment.author) })}
                                size="xs"
                                type="button"
                                variant="ghost"
                              >
                                Reply
                              </ShadcnButton>
                            </div>
                            <Prose onOpen={onProseClick} text={comment.body} tickets={linkedTickets} />
                          </li>
                        )
                      }
                      const EventIcon = activityIcon(entry)
                      return (
                        <li className="kb-event" key={entry.id}>
                          <span aria-hidden="true" className="kb-event__icon">
                            <EventIcon size={16} />
                          </span>
                          <span className="kb-event__text">
                            <strong>{agentName(entry.actor)}</strong> {activitySentence(entry, columns, agentName, keyRef)}
                          </span>
                          <time className="kb-event__time" dateTime={entry.at} title={absoluteTime(entry.at)}>
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

            <aside aria-label="Properties" className={`kb-ticket__aside ${SIDE}`}>
              <SideSection title="Properties">
                <SideRow label="Status">
                  {deleted ? (
                    <span className="tw:flex tw:h-8 tw:items-center tw:gap-2 tw:px-2 tw:text-sm">
                      <StatusIcon step={step(ticket.status)} />
                      {statusLabel(columns, ticket.status)}
                    </span>
                  ) : (
                    <StatusMenu
                      canceledStatus={canceledStatus}
                      columns={columns}
                      doneStatus={doneStatus}
                      onChange={(next) => void patch({ status: next })}
                      quiet
                      status={ticket.status}
                    />
                  )}
                </SideRow>
                <SideRow label="Priority">
                  <PriorityMenu
                    disabled={deleted}
                    onChange={(next) => void patch({ priority: next })}
                    priorities={priorities.length > 0 ? priorities : ['low', 'medium', 'high', 'urgent']}
                    priority={ticket.priority}
                    quiet
                  />
                </SideRow>
                <SideRow label="Assignee">
                  <AssigneePicker
                    agents={agents}
                    disabled={deleted}
                    onChange={(next) => void patch({ assignee: next })}
                    quiet
                    value={ticket.assignee}
                  />
                </SideRow>
              </SideSection>

              <SideSection title="Labels">
                <div className="tw:flex tw:flex-wrap tw:items-center tw:gap-1.5 tw:px-1" ref={labelsRef}>
                  {[...new Set(ticket.labels)].map((label) => (
                    <span className="tw:group/label tw:relative tw:inline-flex" key={label}>
                      <LabelBadge label={label} />
                      {deleted ? null : (
                        <button
                          aria-label={`Remove label ${label}`}
                          className="tw:absolute tw:-top-1.5 tw:-right-1.5 tw:hidden tw:size-4 tw:items-center tw:justify-center tw:rounded-full tw:border tw:border-neutral-100 tw:bg-white tw:text-neutral-500 tw:group-hover/label:flex tw:focus-visible:flex tw:dark:border-neutral-700 tw:dark:bg-neutral-900 tw:dark:text-neutral-300 tw:[&>svg]:size-[10px]"
                          onClick={() => removeLabel(label)}
                          type="button"
                        >
                          <X size={16} />
                        </button>
                      )}
                    </span>
                  ))}
                  <LabelAdder disabled={deleted} id={addLabelId} onAdd={addLabel} own={ticket.labels} suggestions={boardLabels} />
                </div>
              </SideSection>

              <SideSection title="Project">
                <SideRow label="Project">
                  <ProjectPicker
                    disabled={deleted}
                    onChange={(next) => void patch({ project: next })}
                    projects={board?.projects ?? null}
                    quiet
                    value={ticket.project_id}
                  />
                  {ticket.project_id && hasPanels ? (
                    <IconAction
                      className={OPEN}
                      label={projectName ? `Open project ${projectName}` : 'Open project'}
                      onClick={() => host.panels?.open({ pageId: PROJECTS_PAGE_ID, context: { id: ticket.project_id } })}
                      size="icon-xs"
                    >
                      <ArrowUpRight size={16} />
                    </IconAction>
                  ) : null}
                </SideRow>
                {facetOffer('project')}
                <SideRow label="Release">
                  <ReleasePicker
                    disabled={deleted}
                    onChange={(next) => void patch({ release: next })}
                    quiet
                    releases={board?.releases ?? null}
                    value={ticket.release_id}
                  />
                  {ticket.release_id && hasPanels ? (
                    <IconAction
                      className={OPEN}
                      label={releaseName ? `Open release ${releaseName}` : 'Open release'}
                      onClick={() => host.panels?.open({ pageId: RELEASES_PAGE_ID, context: { id: ticket.release_id } })}
                      size="icon-xs"
                    >
                      <ArrowUpRight size={16} />
                    </IconAction>
                  ) : null}
                </SideRow>
                {facetOffer('release')}
                <SideRow label="Parent">
                  <ParentPicker
                    current={parentBrief}
                    disabled={deleted || parentBlocked !== null}
                    onChange={(next) => void patch({ parent: next })}
                    quiet
                    selfId={ticket.id}
                    tickets={boardTickets}
                    value={ticket.parent?.key ?? null}
                  />
                  {ticket.parent && !deleted ? (
                    <IconAction className={OPEN} disabled={busy} label="Detach from parent" onClick={() => void detachFromParent()} size="icon-xs">
                      <X size={16} />
                    </IconAction>
                  ) : null}
                </SideRow>
                {parentBlocked ? (
                  <p className={`tw:m-0 tw:px-2 tw:text-xs ${FAINT}`} id={parentNoteId}>
                    {parentBlocked}
                  </p>
                ) : null}
              </SideSection>

              {ticket.pills?.length ? (
                <SideSection title="Pills">
                  <div className="tw:px-1">
                    <Pills pills={ticket.pills} />
                  </div>
                </SideSection>
              ) : null}

              <SideSection title="Details">
                <dl className={`tw:m-0 tw:flex tw:flex-col tw:gap-1.5 tw:px-2 tw:text-[13px] ${FAINT}`}>
                  <div className="tw:flex tw:gap-1">
                    <dt>Created</dt>
                    <dd className="tw:m-0 tw:text-neutral-700 tw:dark:text-neutral-300">
                      <time dateTime={ticket.created_at} title={absoluteTime(ticket.created_at)}>
                        {ago(ticket.created_at)}
                      </time>{' '}
                      by {agentName(ticket.created_by)}
                    </dd>
                  </div>
                  <div className="tw:flex tw:gap-1">
                    <dt>Updated</dt>
                    <dd className="tw:m-0 tw:text-neutral-700 tw:dark:text-neutral-300">
                      <time dateTime={ticket.updated_at} title={absoluteTime(ticket.updated_at)}>
                        {ago(ticket.updated_at)}
                      </time>
                    </dd>
                  </div>
                  <div className="tw:flex tw:gap-1">
                    <dt>Comments</dt>
                    <dd className="tw:m-0 tw:text-neutral-700 tw:dark:text-neutral-300">{ticket.comments.length}</dd>
                  </div>
                  <div className="tw:flex tw:min-w-0 tw:items-center tw:gap-1">
                    <dt className="tw:shrink-0">Id</dt>
                    <dd className="tw:m-0 tw:flex tw:min-w-0 tw:items-center tw:gap-1">
                      <span className="kb-mono tw:truncate tw:text-xs" title={ticket.id}>
                        {ticket.id}
                      </span>
                      <CopyId text={ticket.id} />
                    </dd>
                  </div>
                </dl>
              </SideSection>
            </aside>
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

/* Tailwind (`tw:`, see tailwind.mjs). The properties sidebar is Linear's: no boxes, no label column — a small
   section heading, then quiet full-width rows of icon and value. */
const FAINT = 'tw:text-neutral-500 tw:dark:text-neutral-400'
const SIDE = [
  'tw:flex tw:flex-col tw:gap-6 tw:pt-1',
  // The pickers' quiet buttons become full-width rows: 32px, left aligned, 14px.
  'tw:[&_.kb-prop]:border-transparent tw:[&_.kb-prop]:bg-transparent tw:[&_.kb-prop]:shadow-none tw:[&_.kb-prop:hover]:bg-neutral-100 tw:dark:[&_.kb-prop:hover]:bg-neutral-900 tw:[&_.kb-prop]:h-8 tw:[&_.kb-prop]:w-full tw:[&_.kb-prop]:max-w-none tw:[&_.kb-prop]:justify-start tw:[&_.kb-prop]:gap-2.5 tw:[&_.kb-prop]:px-2 tw:[&_.kb-prop]:text-sm tw:[&_.kb-prop]:font-normal',
].join(' ')
/** The open/detach action that appears beside a row on hover. */
const OPEN = 'tw:opacity-0 tw:group-hover/row:opacity-100 tw:focus-visible:opacity-100 tw:pointer-coarse:opacity-100'

/** A sidebar section: a 13px grey heading, then its rows. */
function SideSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section aria-label={title} className="tw:flex tw:flex-col tw:gap-0.5">
      <h3 className={`tw:m-0 tw:mb-1.5 tw:px-2 tw:text-[13px] tw:font-medium ${FAINT}`}>{title}</h3>
      {children}
    </section>
  )
}

/** One property: its control, full width, named for assistive tech by the group. */
function SideRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: a fieldset brings its own border and legend layout; this group only names the control
    <div aria-label={label} className="tw:group/row tw:flex tw:min-w-0 tw:items-center tw:gap-1" role="group">
      {children}
    </div>
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
  // Bumped when the worker says its configuration changed: columns and statuses are read again.
  const [configVersion, setConfigVersion] = useState(0)

  useEffect(() => {
    if (contextId) setStoredId(contextId)
  }, [contextId, panelContext?.id, setStoredId])

  useEffect(
    () =>
      subscribeChanges(host.iii, (event) => {
        if (event?.event === CONFIG_EVENT) setConfigVersion((version) => version + 1)
      }),
    [host],
  )

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
  }, [host, configVersion])

  const id = contextId ?? storedId

  if (!id) {
    return (
      <PageShell {...shell}>
        <PageHeader icon={<TicketIcon size={16} />} onClose={onRequestClose} title="Ticket" />
        <PageMain>
          <Empty
            action={
              host.panels?.open ? (
                <ShadcnButton onClick={() => host.panels?.open({ pageId: BOARD_PAGE_ID })} size="sm" type="button" variant="outline">
                  Open the board
                </ShadcnButton>
              ) : undefined
            }
            description="Open one from the board, or mention it in chat."
            icon={TicketIcon}
            title="No ticket selected"
          />
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

