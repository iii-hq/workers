/**
 * The ticket screen (design §6): the root's detail and the `ticket` page. A
 * Page whose body is one List: the ticket's head in its header (the parent
 * strip, the title, the chips, the Done warning, the description in
 * Markdown, the subtasks, the linked tickets) and the activity as its items,
 * comments threaded; the comment composer in the Page's bottom slot.
 */

import {
  type Bound,
  Button,
  Card,
  Chip,
  Column,
  createEffect,
  createList,
  createMemo,
  createSignal,
  EmptyState,
  For,
  Icon,
  IconButton,
  iii,
  itemHandler,
  type JsonObject,
  List,
  ListRow,
  Markdown,
  type MenuAction,
  type NodeRef,
  on,
  onCleanup,
  onMount,
  Page,
  Progress,
  Row,
  Segmented,
  Show,
  Spinner,
  StatusPanel,
  Text,
  TextField,
} from '@iii-dev/native-ui'
import { type ActivityFilter, counts, type TimelineRow, timeline } from '../activity'
import { type Brief, kanban, type Summary, type Ticket } from '../api'
import type { WriteKind } from '../board'
import {
  agoLong,
  hueOf,
  pillIcon,
  plural,
  priorityLabel,
  STEP_ICON,
  STEP_TONE,
  shareText,
  statusLabel,
  statusStep,
} from '../format'
import { type LinkedTicket, linkTicketKeys, TICKET_HREF } from '../links'
import { childOrder, doneWarning, subtaskDefaults } from '../rules'
import {
  agentOf,
  columns,
  meta,
  moveTicket,
  nameOf,
  now,
  OFFLINE,
  onChange,
  online,
  projects,
  readable,
  releases,
  toast,
  wrote,
} from '../state'
import { pick, pickAgent } from './pick'

export function TicketPage(props: { params: JsonObject }) {
  return <TicketView ticketKey={typeof props.params.key === 'string' ? props.params.key : ''} />
}

interface LinkRow {
  ticket: Summary
  relation: string
}

/** One row per linked ticket, whichever side of the link it is on. */
function mergeLinks(mentions: readonly Summary[], mentionedBy: readonly Summary[]): LinkRow[] {
  const rows = new Map<string, LinkRow>()
  for (const t of mentions) rows.set(t.id, { ticket: t, relation: 'mentioned here' })
  for (const t of mentionedBy) rows.set(t.id, { ticket: t, relation: rows.has(t.id) ? 'both ways' : 'mentions this' })
  return [...rows.values()]
}

const FILTERS: ActivityFilter[] = ['all', 'comments', 'changes']
const SUBTASKS_SHOWN = 8

export function TicketView(props: { ticketKey: string }) {
  const [ticket, setTicket] = createSignal<Ticket | null>(null)
  const [error, setError] = createSignal<string | null>(null)
  const [links, setLinks] = createSignal<LinkRow[]>([])
  const [parentView, setParentView] = createSignal<Ticket | null>(null)
  const [filter, setFilter] = createSignal<ActivityFilter>('all')
  const [replyTo, setReplyTo] = createSignal<{ id: string; who: string } | null>(null)
  const [draft, setDraft] = createSignal('')
  const [sending, setSending] = createSignal(false)
  const [busy, setBusy] = createSignal(0)
  const [allSubs, setAllSubs] = createSignal(false)
  const rows = createList<TimelineRow>({ key: 'key', kind: 'kind' })
  const timers = new Map<string, number>()
  let menuButton: NodeRef | undefined
  let composer: NodeRef | undefined
  let statusChip: NodeRef | undefined
  let priorityChip: NodeRef | undefined
  onCleanup(() => {
    for (const timer of timers.values()) clearTimeout(timer)
  })

  /** Shows `next` unless it is another ticket's or older than what is shown (an event can land first). */
  const show = (next: Ticket): boolean => {
    const shown = ticket()
    if (shown) {
      const [at, shownAt] = [Date.parse(next.updated_at), Date.parse(shown.updated_at)]
      if (next.id !== shown.id || at < shownAt || (at === shownAt && next.activity.length < shown.activity.length))
        return false
    }
    setTicket(next)
    setError(null)
    return true
  }

  let asked = 0
  /** Reads the ticket; a `quiet` read keeps what is shown when it fails. */
  const read = async (quiet = false) => {
    const ask = ++asked
    try {
      const next = await kanban.ticket(ticket()?.id ?? props.ticketKey)
      if (ask === asked) show(next)
    } catch (err) {
      if (ask === asked && (!quiet || !ticket())) setError(readable(err))
    }
  }

  let linksAsked = 0
  const readLinks = async () => {
    const ask = ++linksAsked
    try {
      const result = await kanban.links(ticket()?.id ?? props.ticketKey)
      if (ask === linksAsked) setLinks(mergeLinks(result.mentions, result.mentioned_by))
    } catch {
      // An older worker has no links: the section stays hidden.
    }
  }

  let parentAsked = 0
  const readParent = async () => {
    const id = ticket()?.parent_id
    const ask = ++parentAsked
    if (!id) return setParentView(null)
    try {
      const parent = await kanban.ticket(id)
      if (ask === parentAsked) setParentView(parent)
    } catch {
      // The strip shows the parent the ticket carries.
    }
  }

  /** A burst of events is one read. */
  const soon = (name: string, run: () => unknown, ms = 300) => {
    const timer = timers.get(name)
    if (timer !== undefined) clearTimeout(timer)
    timers.set(
      name,
      setTimeout(() => {
        timers.delete(name)
        void run()
      }, ms),
    )
  }

  onMount(() => {
    void read()
    void readLinks()
  })
  createEffect(
    on(
      () => ticket()?.parent_id,
      () => void readParent(),
    ),
  )
  // The text of a ticket names other tickets: the links follow each new version.
  createEffect(
    on(
      () => ticket()?.updated_at,
      (at, before) => {
        if (at && before) soon('links', readLinks, 500)
      },
    ),
  )
  // Back on screen: what changed meanwhile.
  createEffect(
    on(
      () => iii.env.visible,
      (visible, before) => {
        if (visible && before === false) {
          void read(true)
          void readLinks()
        }
      },
    ),
  )

  onCleanup(
    onChange((event) => {
      const own = ticket()
      const s = event.ticket
      if (!s) return soon('self', () => read(true))
      if (!own) {
        if (s.key.toLowerCase() === props.ticketKey.toLowerCase()) soon('self', () => read(true))
        return
      }
      // Itself, or one of its subtasks: the list and the progress are the worker's to count.
      if (s.id === own.id || s.parent_id === own.id || own.children.some((c) => c.id === s.id))
        soon('self', () => read(true))
      // Its parent or a sibling: the strip.
      if (own.parent_id && (s.id === own.parent_id || s.parent_id === own.parent_id)) soon('parent', readParent)
      // A linked ticket, or a text that may cite this one.
      if (links().some((l) => l.ticket.id === s.id) || event.event === 'comment.created') soon('links', readLinks, 500)
    }),
  )

  const deleted = () => !!ticket()?.deleted_at
  const editable = () => !!ticket() && !deleted() && online() && busy() === 0
  const who = { name: nameOf, hue: (id: string) => hueOf(agentOf(id)?.color) }

  /** The tickets the text may link to: the ones the worker says it mentions or is mentioned by. */
  const linked = createMemo<LinkedTicket[]>(() =>
    links().map(({ ticket: t }) => ({ key: t.key, title: t.title, status: statusLabel(columns(), t.status) })),
  )

  createEffect(() => {
    const t = ticket()
    if (!t) return rows.set([])
    const list = linked()
    rows.set(
      timeline(t, filter(), columns(), who, now(), !deleted()).map((row) =>
        row.kind ? { ...row, text: linkTicketKeys(row.text, list) } : row,
      ),
    )
  })

  const follow = (href: string) => {
    if (href.startsWith(TICKET_HREF)) {
      const key = href.slice(TICKET_HREF.length)
      if (linked().some((l) => l.key === key)) void iii.nav.push('ticket', { key })
      else toast("This link can't be opened here")
    } else if (/^(https?:\/\/|mailto:)/i.test(href)) {
      iii.openUrl(href).catch((err: unknown) => toast(readable(err), 'error'))
    } else toast("This link can't be opened here")
  }
  const openTicket = (key: string) => void iii.nav.push('ticket', { key })

  /** One write of this ticket: on screen and on the board when it lands; a failure says why and reads again. */
  const save = async (
    work: () => Promise<Ticket | null>,
    failure = "Couldn't save",
    kind: WriteKind = {},
  ): Promise<boolean> => {
    if (!online()) {
      toast(OFFLINE, 'error')
      return false
    }
    setBusy((b) => b + 1)
    try {
      const next = await work()
      if (next) {
        wrote(next, kind)
        show(next)
      }
      return !!next
    } catch (err) {
      toast(`${failure}: ${readable(err)}`, 'error')
      void read(true)
      return false
    } finally {
      setBusy((b) => b - 1)
    }
  }
  const patch = (changes: JsonObject) => {
    const t = ticket()
    return t ? save(() => kanban.update(t.id, changes)) : Promise.resolve(false)
  }

  /** A write on other tickets (subtasks): one after the other; then this ticket is read again either way. */
  const each = async (children: readonly Brief[], work: (child: Brief) => Promise<Ticket>, moves = false) => {
    if (!online()) return toast(OFFLINE, 'error')
    setBusy((b) => b + 1)
    try {
      for (const child of children) wrote(await work(child), moves ? { moved: true, from: child.status } : {})
    } catch (err) {
      toast(readable(err), 'error')
    } finally {
      setBusy((b) => b - 1)
      void read(true)
    }
  }

  const changeStatus = async () => {
    const t = ticket()
    if (!t) return
    const cols = columns()
    const choice = await iii.ui.actionMenu(statusChip ?? null, {
      title: `Move ${t.key} to`,
      actions: cols.map((c) => ({ label: c.label, checked: c.id === t.status })),
    })
    const column = cols[choice]
    if (column && column.id !== t.status && editable()) {
      const next = await moveTicket(t.id, column.id, undefined, t.status)
      if (next) show(next)
    }
  }

  const changePriority = async () => {
    const t = ticket()
    const priorities = meta()?.priorities ?? []
    if (!t) return
    const values = priorities.includes(t.priority) ? priorities : [...priorities, t.priority]
    const choice = await iii.ui.actionMenu(priorityChip ?? null, {
      title: 'Priority',
      actions: values.map((p) => ({ label: priorityLabel(p), checked: p === t.priority })),
    })
    const priority = values[choice]
    if (priority && priority !== t.priority) await patch({ priority })
  }

  const changeAssignee = async () => {
    const t = ticket()
    if (!t) return
    const picked = await pickAgent(t.assignee)
    if (picked !== undefined && picked !== t.assignee) await patch({ assignee: picked })
  }

  const changeFacet = async (kind: 'project' | 'release') => {
    const t = ticket()
    if (!t) return
    const current = kind === 'project' ? t.project_id : t.release_id
    const picked = await pick({ kind, current })
    if (picked && picked.value !== current) await patch({ [kind]: picked.value })
  }

  const addLabel = async () => {
    const t = ticket()
    if (!t) return
    const picked = await pick({ kind: 'label', exclude: t.labels })
    if (picked?.value) await patch({ add_labels: [picked.value] })
  }

  const labelMenu = async (label: string) => {
    const choice = await iii.ui.actionMenu(null, {
      title: label,
      actions: [{ label: `Remove label ${label}`, icon: 'x', destructive: true }],
    })
    if (choice === 0) await patch({ remove_labels: [label] })
  }

  const edit = async (field: 'title' | 'description') => {
    const t = ticket()
    if (!online()) return toast(OFFLINE, 'error')
    if (!t || !editable()) return
    const result = await iii.nav.push<{ text: string }>('text', {
      title: field === 'title' ? 'Edit title' : 'Edit description',
      initial: field === 'title' ? t.title : t.description,
      multiline: field === 'description',
    })
    const text = typeof result?.text === 'string' ? result.text : null
    if (text === null) return
    const value = field === 'title' ? text.trim() : text
    if ((field === 'title' && !value) || value === (field === 'title' ? t.title : t.description)) return
    await patch({ [field]: value })
  }

  const addSubtask = async () => {
    const t = ticket()
    if (!t) return
    const defaults = subtaskDefaults(t, projects(), releases(), columns()[0]?.id)
    const result = await iii.nav.push<{ key: string }>('new', defaults)
    if (result?.key) {
      toast(`Created ${result.key}`, 'success')
      void read(true)
    }
  }

  const addExisting = async () => {
    const t = ticket()
    if (!t) return
    const picked = await pick({ kind: 'ticket', mode: 'attach', self: t.id })
    if (picked?.value) {
      const child = picked.value
      if (await save(() => kanban.update(child, { parent: t.key }))) void read(true)
    }
  }

  const setParent = async () => {
    const t = ticket()
    if (!t) return
    const picked = await pick({ kind: 'ticket', mode: 'parent', self: t.id })
    if (picked?.value) await patch({ parent: picked.value })
  }

  const detach = async () => {
    const t = ticket()
    if (!t?.parent) return
    const ok = await iii.ui.confirm({
      title: `Detach from ${t.parent.key}?`,
      message: 'It stays on the board as a ticket of its own, with its own project and release.',
      confirm: 'Detach',
    })
    if (ok) await patch({ parent: null })
  }

  const toggleDeleted = async () => {
    const t = ticket()
    if (!t) return
    const restoring = !!t.deleted_at
    const ok = await iii.ui.confirm(
      restoring
        ? { title: 'Restore this ticket?', message: 'It returns to its column on the board.', confirm: 'Restore' }
        : {
            title: 'Delete this ticket?',
            message: 'It disappears from the board but stays in the board file, and its comments are kept.',
            confirm: 'Delete',
            destructive: true,
          },
    )
    if (!ok) return
    await save(
      () => (restoring ? kanban.restore(t.id) : kanban.remove(t.id)),
      restoring ? "Couldn't restore" : "Couldn't delete",
      restoring ? { entered: true } : {},
    )
  }

  const menu = async () => {
    const t = ticket()
    if (!t) return
    const items: [MenuAction, () => unknown][] = []
    if (!t.deleted_at) {
      items.push([{ label: 'Edit title', icon: 'pencil' }, () => edit('title')])
      items.push([{ label: 'Edit description', icon: 'pencil' }, () => edit('description')])
      if (!t.parent_id) {
        items.push([{ label: 'Add subtask', icon: 'circle-plus' }, addSubtask])
        items.push([{ label: 'Add existing ticket…', icon: 'list-tree' }, addExisting])
      }
      if (t.parent) items.push([{ label: `Detach from ${t.parent.key}`, icon: 'list-tree' }, detach])
      else if (!t.children.length) items.push([{ label: 'Set parent…', icon: 'list-tree' }, setParent])
    }
    items.push([
      { label: 'Copy key', icon: 'copy' },
      async () => {
        await iii.clipboard.writeText(t.key)
        toast(`Copied ${t.key}`)
      },
    ])
    items.push([
      { label: 'Share', icon: 'share-2' },
      () => iii.share({ text: shareText(t.key, t.title), title: t.key }),
    ])
    items.push([{ label: 'Refresh', icon: 'refresh-cw' }, () => Promise.all([read(), readLinks()])])
    items.push(
      t.deleted_at
        ? [{ label: 'Restore ticket…', icon: 'rotate-ccw' }, toggleDeleted]
        : [{ label: 'Delete ticket…', icon: 'trash', destructive: true }, toggleDeleted],
    )
    const choice = await iii.ui.actionMenu(menuButton ?? null, {
      title: t.key,
      subtitle: t.title,
      actions: items.map(([action]) => action),
    })
    const run = items[choice]?.[1]
    if (!run) return
    try {
      await run()
    } catch (err) {
      toast(readable(err), 'error')
    }
  }

  const reply = (item: TimelineRow) => {
    if (!item.canReply) return
    setReplyTo({ id: item.root, who: item.who })
    if (composer) iii.ui.focus(composer)
  }

  const commentMenu = async (item: TimelineRow) => {
    if (!item.comment) return
    const choice = await iii.ui.actionMenu(null, {
      title: item.who,
      actions: [
        ...(item.canReply ? [{ label: 'Reply', icon: 'corner-down-right' }] : []),
        { label: 'Copy text', icon: 'copy' },
      ],
    })
    const body = ticket()?.comments.find((c) => c.id === item.comment)?.body ?? ''
    if (choice === 0 && item.canReply) reply(item)
    else if (choice >= 0) {
      await iii.clipboard.writeText(body)
      toast('Copied the comment')
    }
  }

  const send = async () => {
    const t = ticket()
    const sent = draft()
    const body = sent.trim()
    if (!t || !body || sending()) return
    if (!online()) return toast(OFFLINE, 'error')
    setSending(true)
    try {
      const next = await kanban.comment(t.id, body, replyTo()?.id ?? null)
      wrote(next)
      show(next)
      setReplyTo(null)
      // What was sent leaves the field; what was typed meanwhile stays.
      setDraft((now) => (now.startsWith(sent) ? now.slice(sent.length).trimStart() : now))
    } catch (err) {
      toast(`Couldn't send the comment: ${readable(err)}`, 'error')
    } finally {
      setSending(false)
    }
  }

  const step = () => {
    const t = ticket()
    const m = meta()
    return t && m ? statusStep(m.columns, t.status, m.done, m.canceled) : 'unknown'
  }
  const hiddenChanges = () => {
    const t = ticket()
    return t ? t.activity_total - t.activity.length : 0
  }
  const tally = () => {
    const t = ticket()
    return t ? counts(t) : { comments: 0, changes: 0 }
  }

  return (
    <Page
      testTag="ticket"
      title={ticket()?.key ?? props.ticketKey}
      subtitle={ticket() ? (deleted() ? 'Deleted' : statusLabel(columns(), ticket()?.status ?? '')) : null}
      subtitleMono={false}
      actions={
        <IconButton
          testTag="ticket-menu"
          ref={(node) => {
            menuButton = node
          }}
          visible={!!ticket()}
          icon="ellipsis-vertical"
          label="Ticket menu"
          onPress={() => void menu()}
        />
      }
      top={
        <Show when={deleted()}>
          <StatusPanel
            testTag="deleted"
            variant="warn"
            headline="This ticket was deleted"
            detail="It is hidden from the board but the row is still in the board file."
            padding={[8, 12]}
            action={
              <Button testTag="restore" small enabled={online() && busy() === 0} onPress={() => void toggleDeleted()}>
                Restore
              </Button>
            }
          />
        </Show>
      }
      bottom={
        <Show when={ticket() && !deleted()}>
          <Column gap={4} padding={[6, 12]}>
            <Show when={replyTo()}>
              {(r: () => { id: string; who: string }) => (
                <Row testTag="replying" gap={8}>
                  <Icon name="corner-down-right" size={14} tone="muted" />
                  <Text role="small" tone="muted" weight={1}>{`Replying to ${r().who}`}</Text>
                  <Button testTag="cancel-reply" small kind="ghost" onPress={() => setReplyTo(null)}>
                    Cancel
                  </Button>
                </Row>
              )}
            </Show>
            <Row gap={8}>
              <TextField
                testTag="composer"
                ref={(node) => {
                  composer = node
                }}
                weight={1}
                lines={2}
                value={draft()}
                onChange={setDraft}
                placeholder={replyTo() ? 'Write a reply…' : 'Add a comment…'}
              />
              <Show when={!sending()} fallback={<Spinner testTag="sending" />}>
                <IconButton
                  testTag="send"
                  icon="send"
                  label={replyTo() ? 'Reply' : 'Comment'}
                  tone="accent"
                  enabled={online() && draft().trim().length > 0}
                  onPress={() => void send()}
                />
              </Show>
            </Row>
          </Column>
        </Show>
      }
    >
      <Show
        when={ticket()}
        fallback={
          <Show when={error()} fallback={<Spinner testTag="ticket-loading" padding={32} />}>
            {(message: () => string) => (
              <EmptyState
                testTag="ticket-error"
                icon="triangle-alert"
                title="That ticket could not be opened"
                description={message()}
                actions={
                  <Button testTag="ticket-retry" enabled={online()} onPress={() => void read()}>
                    Retry
                  </Button>
                }
              />
            )}
          </Show>
        }
      >
        {(t: () => Ticket) => (
          <List
            testTag="activity"
            weight={1}
            source={rows}
            gap={8}
            contentPadding={[0, 12, 16, 12]}
            header={
              <Column gap={12} padding={[12, 4]}>
                <Show when={t().parent}>
                  {(p: () => Brief) => {
                    const progress = () => {
                      const v = parentView()
                      return v && v.id === p().id && v.progress.total ? `${v.progress.done}/${v.progress.total}` : ''
                    }
                    return (
                      <Card testTag="parent" gap={4} onPress={() => openTicket(p().key)}>
                        <Row gap={6}>
                          <Icon name="corner-down-right" size={14} tone="muted" />
                          <Text role="small" tone="muted">
                            Subtask of
                          </Text>
                          <Text role="idSmall">{p().key}</Text>
                          <Text role="small" weight={1} maxLines={1}>
                            {p().deleted_at ? `${p().title} (deleted)` : p().title}
                          </Text>
                          <Text role="small" tone="muted" visible={!!progress()}>
                            {progress()}
                          </Text>
                        </Row>
                      </Card>
                    )
                  }}
                </Show>
                <Text testTag="title" role="title" onPress={() => void edit('title')}>
                  {t().title || t().key}
                </Text>
                <Row gap={6} wrap>
                  <Chip
                    testTag="status"
                    ref={(node) => {
                      statusChip = node
                    }}
                    icon={STEP_ICON[step()]}
                    tone={STEP_TONE[step()]}
                    text={deleted() ? 'Deleted' : statusLabel(columns(), t().status)}
                    enabled={editable()}
                    onPress={() => void changeStatus()}
                  />
                  <Chip
                    testTag="priority"
                    ref={(node) => {
                      priorityChip = node
                    }}
                    text={priorityLabel(t().priority)}
                    tone={priorityTone(t().priority, meta()?.priorities ?? [])}
                    enabled={editable()}
                    onPress={() => void changePriority()}
                  />
                  <Chip
                    testTag="assignee"
                    icon={t().assignee ? 'bot' : 'circle-dashed'}
                    hue={hueOf(agentOf(t().assignee)?.color)}
                    text={t().assignee ? nameOf(t().assignee as string) : 'Unassigned'}
                    enabled={editable()}
                    onPress={() => void changeAssignee()}
                  />
                  <Chip
                    testTag="project"
                    icon="folder"
                    hue={hueOf(projects().find((p) => p.id === t().project_id)?.color)}
                    text={
                      t().project_id
                        ? (projects().find((p) => p.id === t().project_id)?.name ?? 'Project')
                        : 'No project'
                    }
                    enabled={editable()}
                    onPress={() => void changeFacet('project')}
                  />
                  <Chip
                    testTag="release"
                    icon="rocket"
                    text={
                      t().release_id
                        ? (releases().find((r) => r.id === t().release_id)?.name ?? 'Release')
                        : 'No release'
                    }
                    enabled={editable()}
                    onPress={() => void changeFacet('release')}
                  />
                </Row>
                <Row gap={6} wrap>
                  <For each={t().labels}>
                    {(label) => (
                      <Chip text={label} selected enabled={editable()} onPress={() => void labelMenu(label)} />
                    )}
                  </For>
                  <Chip
                    testTag="add-label"
                    icon="plus"
                    text="Label"
                    enabled={editable()}
                    onPress={() => void addLabel()}
                  />
                  <For each={t().pills}>{(pill) => <Chip icon={pillIcon(pill.icon)} text={pill.text} />}</For>
                </Row>
                <Text testTag="meta" role="small" tone="muted">
                  {`Updated ${agoLong(t().updated_at, now())} · Created ${agoLong(t().created_at, now())} by ${nameOf(t().created_by)}`}
                </Text>
                <DoneWarning ticket={t()} busy={!editable()} onEach={each} onShow={show} />
                <Row gap={8}>
                  <Text role="micro" tone="muted" weight={1}>
                    Description
                  </Text>
                  <Button
                    testTag="edit-description"
                    small
                    kind="ghost"
                    enabled={editable()}
                    onPress={() => void edit('description')}
                  >
                    Edit
                  </Button>
                </Row>
                <Show
                  when={t().description.trim()}
                  fallback={
                    <Text role="small" tone="faint">
                      No description yet.
                    </Text>
                  }
                >
                  <Markdown testTag="description" selectable onLink={follow}>
                    {linkTicketKeys(t().description, linked())}
                  </Markdown>
                </Show>
                <Show when={!t().parent_id && t().children.length > 0}>
                  <Subtasks
                    ticket={t()}
                    all={allSubs()}
                    onAll={() => setAllSubs(true)}
                    onOpen={openTicket}
                    onAdd={() => void addSubtask()}
                    enabled={editable()}
                  />
                </Show>
                <LinkedTickets
                  rows={links().filter(
                    (l) => l.ticket.id !== t().parent_id && !t().children.some((c) => c.id === l.ticket.id),
                  )}
                  onOpen={openTicket}
                />
                <Row gap={8}>
                  <Text role="micro" tone="muted" weight={1}>
                    Activity
                  </Text>
                  <Segmented
                    testTag="activity-filter"
                    compact
                    options={['All', `Comments ${tally().comments}`, `Changes ${tally().changes}`]}
                    selected={FILTERS.indexOf(filter())}
                    onSelect={(i: number) => setFilter(FILTERS[i] ?? 'all')}
                  />
                </Row>
              </Column>
            }
            templates={[
              (row) => (
                <Row gap={8} padding={[4, 4]}>
                  <Icon name={row.icon} size={16} tone="muted" />
                  <Text role="small" tone="secondary" weight={1} text={row.text} />
                  <Text role="small" tone="faint" text={row.time} />
                </Row>
              ),
              (row) => <CommentCard row={row} onReply={reply} onMenu={commentMenu} onLink={follow} />,
              (row) => (
                <Row gap={6} padding={[0, 0, 0, 16]}>
                  <Icon name="corner-down-right" size={14} tone="faint" />
                  <Column weight={1}>
                    <CommentCard row={row} onReply={reply} onMenu={commentMenu} onLink={follow} />
                  </Column>
                </Row>
              ),
            ]}
            empty={
              <Text role="small" tone="faint" padding={[8, 4]}>
                {filter() === 'all'
                  ? 'Nothing has happened yet.'
                  : filter() === 'comments'
                    ? 'No comments yet.'
                    : 'No changes yet.'}
              </Text>
            }
            footer={
              <Show when={filter() !== 'comments' && hiddenChanges() > 0}>
                <Text role="small" tone="faint" padding={[8, 4]}>
                  {`${plural(hiddenChanges(), 'older change')} ${hiddenChanges() === 1 ? 'is' : 'are'} in the console`}
                </Text>
              </Show>
            }
          />
        )}
      </Show>
    </Page>
  )
}

/** The priority chip's tone: the most urgent configured priority (the last) is danger, the next warning. */
function priorityTone(priority: string, priorities: readonly string[]): 'neutral' | 'warning' | 'danger' {
  const i = priorities.indexOf(priority)
  const fromTop = i < 0 ? -1 : priorities.length - 1 - i
  return fromTop === 0 ? 'danger' : fromTop === 1 ? 'warning' : 'neutral'
}

/** A comment (template 1) or a reply (inside template 2): the author with the agent's color, the time, Reply, the body. */
function CommentCard(props: {
  row: Bound<TimelineRow>
  onReply: (item: TimelineRow) => void
  onMenu: (item: TimelineRow) => void
  onLink: (href: string) => void
}) {
  const r = props.row
  return (
    <Card gap={6} onLongPress={(item: TimelineRow) => void props.onMenu(item)}>
      <Row gap={8}>
        <Icon name={r.icon} hue={r.hue} size={16} />
        <Text role="smallStrong" weight={1} text={r.who} />
        <Text role="small" tone="faint" text={r.time} />
        <Button small kind="ghost" visible={r.canReply} onPress={(item: TimelineRow) => props.onReply(item)}>
          Reply
        </Button>
      </Row>
      <Markdown
        dense
        selectable
        text={r.text}
        onLink={itemHandler((_item: TimelineRow, href: string) => props.onLink(href))}
      />
    </Card>
  )
}

/** Done with open subtasks (design §6.5): the three ways out of the console. */
function DoneWarning(props: {
  ticket: Ticket
  busy: boolean
  onEach: (children: readonly Brief[], work: (child: Brief) => Promise<Ticket>, moves?: boolean) => Promise<void>
  onShow: (t: Ticket) => boolean
}) {
  const warning = () => {
    const m = meta()
    return m ? doneWarning(props.ticket, m.done, m.canceled, m.columns) : null
  }
  const doneLabel = () => statusLabel(columns(), meta()?.done ?? '')
  const moveBack = async (back: string) => {
    const next = await moveTicket(props.ticket.id, back, undefined, props.ticket.status)
    if (next) props.onShow(next)
  }
  const closeAll = async (open: readonly Brief[]) => {
    const ok = await iii.ui.confirm({
      title: `Close ${plural(open.length, 'subtask')}?`,
      message: `They move to ${doneLabel()}. ${props.ticket.key} stays where it is.`,
      confirm: `Close ${plural(open.length, 'subtask')}`,
    })
    if (ok) await props.onEach(open, (c) => kanban.move(c.id, meta()?.done ?? ''), true)
  }
  const detachAll = async (open: readonly Brief[]) => {
    const ok = await iii.ui.confirm({
      title: `Detach ${plural(open.length, 'subtask')}?`,
      message: `They stop being subtasks of ${props.ticket.key} and stay on the board as tickets of their own.`,
      confirm: `Detach ${plural(open.length, 'subtask')}`,
    })
    if (ok) await props.onEach(open, (c) => kanban.update(c.id, { parent: null }))
  }
  return (
    <Show when={warning()}>
      {(w: () => { open: Brief[]; back: { id: string; label: string } | null }) => (
        <Column gap={8}>
          <StatusPanel
            testTag="done-warning"
            variant="warn"
            headline={`${doneLabel()} with ${plural(w().open.length, 'open subtask')}`}
            detail={`${props.ticket.key} is in ${doneLabel()} while ${w().open.length} of its ${props.ticket.progress.total} subtasks are not. Progress shows ${props.ticket.progress.done} of ${props.ticket.progress.total} because only the ${doneLabel()} column counts.`}
          />
          <Row gap={6} wrap>
            <Show when={w().back}>
              {(back: () => { id: string; label: string }) => (
                <Button testTag="move-back" small enabled={!props.busy} onPress={() => void moveBack(back().id)}>
                  {`Move back to ${back().label}`}
                </Button>
              )}
            </Show>
            <Button testTag="close-subtasks" small enabled={!props.busy} onPress={() => void closeAll(w().open)}>
              {`Close ${plural(w().open.length, 'subtask')}…`}
            </Button>
            <Button testTag="detach-subtasks" small enabled={!props.busy} onPress={() => void detachAll(w().open)}>
              {`Detach ${plural(w().open.length, 'subtask')}…`}
            </Button>
          </Row>
        </Column>
      )}
    </Show>
  )
}

function Subtasks(props: {
  ticket: Ticket
  all: boolean
  onAll: () => void
  onOpen: (key: string) => void
  onAdd: () => void
  enabled: boolean
}) {
  const m = () => meta()
  const ordered = () => childOrder(props.ticket.children, m()?.done ?? '', m()?.canceled ?? null)
  const shown = () => (props.all ? ordered() : ordered().slice(0, SUBTASKS_SHOWN))
  const progress = () => props.ticket.progress
  return (
    <Column testTag="subtasks" gap={4}>
      <Row gap={8}>
        <Text role="micro" tone="muted" weight={1}>
          Subtasks
        </Text>
        <Text role="small" tone="muted">{`${progress().done} of ${progress().total}`}</Text>
      </Row>
      <Progress value={progress().total ? progress().done / progress().total : 0} tone="success" />
      <For each={shown()}>
        {(child) => {
          const s = () => statusStep(columns(), child.status, m()?.done ?? '', m()?.canceled ?? null)
          return (
            <ListRow
              icon={STEP_ICON[s()]}
              title={`${child.key}  ${child.title}`}
              subtitle={statusLabel(columns(), child.status)}
              onPress={() => props.onOpen(child.key)}
            />
          )
        }}
      </For>
      <Row gap={8}>
        <Show when={!props.all && ordered().length > SUBTASKS_SHOWN}>
          <Button testTag="all-subtasks" small kind="ghost" onPress={props.onAll}>
            {`Show all ${ordered().length}`}
          </Button>
        </Show>
        <Button testTag="add-subtask" small kind="ghost" icon="plus" enabled={props.enabled} onPress={props.onAdd}>
          Add subtask
        </Button>
      </Row>
    </Column>
  )
}

function LinkedTickets(props: { rows: LinkRow[]; onOpen: (key: string) => void }) {
  return (
    <Show when={props.rows.length > 0}>
      <Column testTag="links" gap={4}>
        <Text role="micro" tone="muted">
          Linked tickets
        </Text>
        <For each={props.rows}>
          {(row) => {
            const m = meta()
            return (
              <ListRow
                icon={STEP_ICON[statusStep(columns(), row.ticket.status, m?.done ?? '', m?.canceled ?? null)]}
                title={`${row.ticket.key}  ${row.ticket.title}`}
                subtitle={`${statusLabel(columns(), row.ticket.status)} · ${row.relation}`}
                onPress={() => props.onOpen(row.ticket.key)}
              />
            )
          }}
        </For>
      </Column>
    </Show>
  )
}
