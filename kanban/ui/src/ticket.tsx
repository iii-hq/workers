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
  Select,
  Selector,
  Skeleton,
  StatusPanel,
  type Host,
  type PageRenderProps,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import {
  AlertIcon,
  BackIcon,
  ClockIcon,
  ColumnsIcon,
  EditIcon,
  MessageIcon,
  RefreshIcon,
  TICKET_PAGE_ID,
  TicketIcon,
  TrashIcon,
  api,
  errorMessage,
  priorityTone,
  relativeTime,
  statusLabel,
  subscribeChanges,
  useContainerNarrow,
  usePaneState,
  type Activity,
  type AgentProfile,
  type Change,
  type Column,
  type Ticket,
} from './shared'

function readContextId(context: unknown): string | null {
  if (!context || typeof context !== 'object') return null
  const value = (context as { id?: unknown }).id
  return typeof value === 'string' && value.trim() ? value : null
}

const FIELD_LABELS: Record<string, string> = {
  title: 'the title',
  description: 'the description',
  status: 'the status',
  priority: 'the priority',
  assignee: 'the assignee',
  labels: 'the labels',
}

function describeChange(change: Change, columns: Column[]): string {
  const field = FIELD_LABELS[change.field] ?? change.field
  if (change.field === 'status') {
    return `moved this from ${statusLabel(columns, String(change.from))} to ${statusLabel(columns, String(change.to))}`
  }
  if (change.field === 'assignee') {
    return change.to ? `assigned this to ${String(change.to)}` : 'unassigned this'
  }
  return `changed ${field}`
}

function activitySentence(entry: Activity, columns: Column[]): string {
  switch (entry.type) {
    case 'ticket.created':
      return `created this in ${statusLabel(columns, String(entry.changes?.[0]?.to ?? ''))}`
    case 'ticket.moved':
      return entry.changes && entry.changes[0] ? describeChange(entry.changes[0], columns) : 'moved this'
    case 'ticket.updated':
      return (entry.changes ?? []).map((change) => describeChange(change, columns)).join(', ') || 'updated this'
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

type ComposerProps = {
  inputId: string
  replyTo: { id: string; author: string } | null
  busy: boolean
  onSubmit: (body: string) => void
  onCancelReply: () => void
}

function Composer({ inputId, replyTo, busy, onSubmit, onCancelReply }: ComposerProps) {
  const [body, setBody] = useState('')
  useEffect(() => {
    setBody('')
  }, [replyTo?.id])
  const submit = () => {
    const trimmed = body.trim()
    if (!trimmed) return
    onSubmit(trimmed)
    setBody('')
  }
  return (
    <div className="kanban-composer">
      {replyTo ? (
        <div className="kanban-composer__reply">
          <span>Replying to {replyTo.author}</span>
          <Button onClick={onCancelReply} size="sm" variant="ghost">
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
            onCancelReply()
            return
          }
          if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
            event.preventDefault()
            submit()
          }
        }}
        placeholder={replyTo ? 'Write a reply…' : 'Add a comment…'}
        rows={3}
        value={body}
      />
      <div className="kanban-composer__foot">
        <span className="kanban-composer__hint">Markdown supported · ⌘↵ to send</span>
        <Button disabled={busy || body.trim().length === 0} onClick={submit} size="sm" variant="primary">
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
  onBack?: () => void
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
  onBack,
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
  const [busy, setBusy] = useState(false)
  const [narrowRef, narrow] = useContainerNarrow(880)
  const requestRef = useRef(0)

  const load = useCallback(async () => {
    const request = requestRef.current + 1
    requestRef.current = request
    setLoading(true)
    setError(null)
    try {
      const result = await api.ticket(host.iii, ticketId)
      if (requestRef.current !== request) return
      setTicket(result)
    } catch (caught) {
      if (requestRef.current !== request) return
      setError(errorMessage(caught))
    } finally {
      if (requestRef.current === request) setLoading(false)
    }
  }, [host, ticketId])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    return subscribeChanges(host.iii, (event) => {
      if (!event?.ticket) return
      if (event.ticket.id !== ticket?.id && event.ticket.key !== ticketId) return
      setTicket(event.ticket)
    })
  }, [host, ticket?.id, ticketId])

  useEffect(() => {
    if (!setDirty) return
    const descriptionDirty = editingDescription && draft !== (ticket?.description ?? '')
    const titleDirty = editingTitle && titleDraft.trim() !== (ticket?.title ?? '')
    setDirty(descriptionDirty || titleDirty)
  }, [editingDescription, draft, ticket?.description, editingTitle, titleDraft, ticket?.title, setDirty])

  const composerId = useId()
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
      if (!ticket) return false
      setBusy(true)
      setActionError(null)
      try {
        const updated = await api.update(host.iii, { id: ticket.key, ...changes, actor: 'user' })
        setTicket(updated)
        return true
      } catch (caught) {
        setActionError(errorMessage(caught))
        void load()
        return false
      } finally {
        setBusy(false)
      }
    },
    [host, ticket, load],
  )

  const saveDescription = async () => {
    if (await patch({ description: draft })) setEditingDescription(false)
  }

  const saveTitle = async () => {
    const next = titleDraft.trim()
    if (!ticket || !next) return
    if (next === ticket.title || (await patch({ title: next }))) setEditingTitle(false)
  }

  const submitComment = async (body: string) => {
    if (!ticket) return
    setBusy(true)
    setActionError(null)
    try {
      const result = await api.comment(host.iii, {
        ticket_id: ticket.key,
        body,
        parent_id: replyTo?.id ?? null,
        author: 'user',
      })
      setTicket(result.ticket)
      setReplyTo(null)
    } catch (caught) {
      setActionError(errorMessage(caught))
    } finally {
      setBusy(false)
    }
  }

  const toggleDeleted = async () => {
    if (!ticket) return
    setBusy(true)
    setActionError(null)
    try {
      const updated = ticket.deleted_at
        ? await api.restore(host.iii, { id: ticket.key, actor: 'user' })
        : await api.remove(host.iii, { id: ticket.key, actor: 'user' })
      setTicket(updated)
      onDeleted?.(Boolean(updated.deleted_at))
      setConfirming(false)
    } catch (caught) {
      setActionError(errorMessage(caught))
    } finally {
      setBusy(false)
    }
  }

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
        label: `${agent.logo ? `${agent.logo} ` : ''}${agent.name}`,
        description: agent.description ?? undefined,
      })),
    [agents],
  )

  const priorityOptions = useMemo(() => {
    const values = priorities.length > 0 ? [...priorities] : ['low', 'medium', 'high', 'urgent']
    if (ticket && !values.includes(ticket.priority)) values.push(ticket.priority)
    return values.map((value) => ({ value, label: value }))
  }, [priorities, ticket])

  if (loading && !ticket) {
    return (
      <PageShell className="kanban-shell">
        <PageHeader icon={<TicketIcon />} onClose={onRequestClose} title="Ticket" />
        <PageMain>
          <div className="kanban-record" ref={narrowRef}>
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
      <PageShell className="kanban-shell">
        <PageHeader
          actions={
            onBack ? (
              <IconButton label="Back to the board" onClick={onBack} variant="ghost">
                <BackIcon />
              </IconButton>
            ) : null
          }
          icon={<TicketIcon />}
          onClose={onRequestClose}
          title="Ticket"
        />
        <PageMain>
          <div className="kanban-pad">
            <StatusPanel
              detail={error ?? `No ticket matches ${ticketId}.`}
              headline="That ticket could not be opened"
              icon={<AlertIcon />}
              variant="alert"
            />
            <Button onClick={() => void load()} variant="pill">
              Retry
            </Button>
          </div>
        </PageMain>
      </PageShell>
    )
  }

  const deleted = Boolean(ticket.deleted_at)

  return (
    <PageShell className="kanban-shell">
      <PageHeader
        actions={
          <>
            {onBack ? (
              <IconButton label="Back to the board" onClick={onBack} variant="ghost">
                <BackIcon />
              </IconButton>
            ) : null}
            {onBack && host.panels?.open ? (
              <IconButton
                label="Open in its own pane"
                onClick={() => {
                  host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id: ticket.key } })
                  onBack()
                }}
                variant="ghost"
              >
                <ColumnsIcon />
              </IconButton>
            ) : null}
            <IconButton label="Refresh ticket" onClick={() => void load()} variant="ghost">
              <RefreshIcon />
            </IconButton>
            <IconButton
              className="kanban-danger"
              label={deleted ? 'Restore ticket' : 'Delete ticket'}
              onClick={() => setConfirming(true)}
              variant="ghost"
            >
              <TrashIcon />
            </IconButton>
          </>
        }
        description={`${ticket.key} · ${ticket.title}`}
        icon={<TicketIcon />}
        onClose={onRequestClose}
        title="Ticket"
      />
      <PageMain>
        <div
          className="kanban-record"
          data-autofocus=""
          data-narrow={narrow ? 'true' : undefined}
          ref={narrowRef}
          tabIndex={-1}
        >
          <div className="kanban-record__grid">
            <div className="kanban-record__head" data-area="head">
              <div className="kanban-record__eyebrow">
                <span className="kanban-mono">{ticket.key}</span>
                <Badge variant={deleted ? 'alert' : 'default'}>
                  {deleted ? 'Deleted' : statusLabel(columns, ticket.status)}
                </Badge>
                <Chip tone={priorityTone(ticket.priority)}>{ticket.priority}</Chip>
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
                  <Button onClick={() => setEditingTitle(false)} size="sm" variant="ghost">
                    Cancel
                  </Button>
                  <Button disabled={busy || titleDraft.trim().length === 0} onClick={() => void saveTitle()} size="sm" variant="primary">
                    Save
                  </Button>
                </div>
              ) : (
                <div className="kanban-record__titlebar">
                  <h1 className="kanban-record__title">{ticket.title}</h1>
                  {deleted ? null : (
                    <IconButton label="Edit title" onClick={startEditingTitle} variant="ghost">
                      <EditIcon />
                    </IconButton>
                  )}
                </div>
              )}
              <div className="kanban-record__meta">
                <ClockIcon />
                <span title={new Date(ticket.updated_at).toLocaleString()}>updated {relativeTime(ticket.updated_at)}</span>
                <span className="kanban-dot" />
                <span>created {relativeTime(ticket.created_at)} by {ticket.created_by}</span>
              </div>
            </div>

            <aside className="kanban-record__props" data-area="props">
              <div className="kanban-props">
                <div className="kanban-prop">
                  <span className="kanban-prop__label">Status</span>
                  <Select
                    appearance="inline"
                    onChange={(next) => void patch({ status: next })}
                    options={columns.map((column) => ({ value: column.id, label: column.label }))}
                    value={ticket.status}
                  />
                </div>
                <div className="kanban-prop">
                  <span className="kanban-prop__label">Priority</span>
                  <Select
                    appearance="inline"
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
                    emptyLabel="Unassigned"
                    onChange={(next) => void patch({ assignee: next })}
                    onClear={() => void patch({ assignee: null })}
                    options={agentOptions}
                    placeholder="Unassigned"
                    value={ticket.assignee ?? undefined}
                  />
                </div>
                {ticket.labels.length > 0 ? (
                  <div className="kanban-prop kanban-prop--static">
                    <span className="kanban-prop__label">Labels</span>
                    <span className="kanban-prop__chips">
                      {ticket.labels.map((label) => (
                        <Chip key={label}>{label}</Chip>
                      ))}
                    </span>
                  </div>
                ) : null}
                <div className="kanban-prop kanban-prop--static">
                  <span className="kanban-prop__label">Ticket id</span>
                  <span className="kanban-mono kanban-prop__value">{ticket.id}</span>
                </div>
                <div className="kanban-prop kanban-prop--static">
                  <span className="kanban-prop__label">Comments</span>
                  <span className="kanban-prop__value">{ticket.comments.length}</span>
                </div>
              </div>
            </aside>

            <section className="kanban-record__body" data-area="body">
              {deleted ? (
                <StatusPanel
                  className="kanban-record__banner"
                  detail="It is hidden from the board but the row is still in the board file."
                  headline="This ticket was deleted"
                  icon={<AlertIcon />}
                  variant="warn"
                />
              ) : null}
              {actionError ? (
                <StatusPanel
                  className="kanban-record__banner"
                  detail={actionError}
                  headline="That change was not saved"
                  icon={<AlertIcon />}
                  variant="alert"
                />
              ) : null}

              <div className="kanban-section__head">
                <h2 className="kanban-section__title">Description</h2>
                {editingDescription ? (
                  <div className="kanban-section__actions">
                    <Button
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
                      disabled={busy}
                      onClick={() => void saveDescription()}
                      size="sm"
                      variant="primary"
                    >
                      Save
                    </Button>
                  </div>
                ) : (
                  <Button disabled={deleted} onClick={startEditing} size="sm" variant="ghost">
                    Edit
                  </Button>
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
                <div className="kanban-prose">
                  <Markdown>{ticket.description}</Markdown>
                </div>
              ) : (
                <p className="kanban-muted">No description yet.</p>
              )}
            </section>

            <section className="kanban-record__activity" data-area="activity">
              <div className="kanban-section__head">
                <h2 className="kanban-section__title">Activity</h2>
                <span className="kanban-section__count">{timeline.length} entries</span>
              </div>
              {timeline.length === 0 ? (
                <p className="kanban-muted">Nothing has happened yet.</p>
              ) : (
                <ol className="kanban-timeline">
                  {timeline.map(({ entry, comment }) => {
                    if (comment) {
                      return (
                        <li
                          className="kanban-comment"
                          data-reply={comment.parent_id ? 'true' : undefined}
                          key={entry.id}
                        >
                          <div className="kanban-comment__head">
                            <span className="kanban-avatar" data-color={agents.find((a) => a.id === comment.author)?.color ?? undefined}>
                              {agents.find((a) => a.id === comment.author)?.logo ?? comment.author.slice(0, 1).toUpperCase()}
                            </span>
                            <span className="kanban-comment__author">
                              {agents.find((a) => a.id === comment.author)?.name ?? comment.author}
                            </span>
                            <time
                              className="kanban-mono kanban-comment__time"
                              dateTime={comment.created_at}
                              title={new Date(comment.created_at).toLocaleString()}
                            >
                              {relativeTime(comment.created_at)}
                            </time>
                            <Button
                              className="kanban-comment__reply"
                              disabled={deleted}
                              onClick={() => setReplyTo({ id: comment.id, author: comment.author })}
                              size="sm"
                              variant="ghost"
                            >
                              Reply
                            </Button>
                          </div>
                          <div className="kanban-prose">
                            <Markdown>{comment.body}</Markdown>
                          </div>
                        </li>
                      )
                    }
                    return (
                      <li className="kanban-event" key={entry.id}>
                        <span className="kanban-event__icon">
                          <MessageIcon />
                        </span>
                        <span className="kanban-event__text">
                          <strong>{entry.actor}</strong> {activitySentence(entry, columns)}
                        </span>
                        <time
                          className="kanban-mono kanban-event__time"
                          dateTime={entry.at}
                          title={new Date(entry.at).toLocaleString()}
                        >
                          {relativeTime(entry.at)}
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
                  onSubmit={(body) => void submitComment(body)}
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
  const [agents, setAgents] = useState<AgentProfile[]>([])

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
      <PageShell className="kanban-shell">
        <PageHeader icon={<TicketIcon />} onClose={onRequestClose} title="Ticket" />
        <PageMain>
          <div className="kanban-pad">
            <EmptyState
              description="Open one from the board, or reference it in chat with kanban::ticket::get."
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
      commands={commands}
      host={host}
      onRequestClose={onRequestClose}
      priorities={priorities}
      setDirty={setDirty}
      ticketId={id}
    />
  )
}

