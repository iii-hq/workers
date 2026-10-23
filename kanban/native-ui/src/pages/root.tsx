/**
 * The root page (design §2–§5, §7): "Kanban" over a ListDetail whose list
 * fills the width (listFill) with the board's Lanes, one List of cards per
 * column, and whose detail is the open ticket. The host arranges it: one lane
 * per page under tabs on a phone, the lanes side by side on a wide screen,
 * and back to one lane (the open card's) beside the ticket.
 */

import {
  Badge,
  Button,
  Card,
  Chip,
  Column,
  createEffect,
  createList,
  createMemo,
  createRoot,
  createSignal,
  EmptyState,
  For,
  Icon,
  IconButton,
  iii,
  Lanes,
  List,
  ListDetail,
  type NodeRef,
  on,
  onCleanup,
  onMount,
  Page,
  Row,
  Show,
  Spinner,
  StatusDot,
  StatusPanel,
  Text,
} from '@iii-dev/native-ui'
import { kanban, type Summary } from '../api'
import { allRows, dropIndex, type Lane, partial, subtaskStats } from '../board'
import type { CardRow } from '../cards'
import { createCardCache, createFeeder } from '../feed'
import { activeCount, EMPTY_FILTERS, matches } from '../filters'
import { count, priorityLabel, shareText } from '../format'
import {
  agentOf,
  agents,
  complete,
  filtering,
  filters,
  goLive,
  lane,
  lanes,
  load,
  loadAgents,
  loadError,
  loaded,
  matchContext,
  meta,
  more,
  moveTicket,
  online,
  openCard,
  paging,
  readable,
  restore,
  selected,
  setFilters,
  setLane,
  setNow,
  setSelected,
  stampVisit,
  toast,
  wrote,
} from '../state'
import { pickAgent } from './pick'
import { TicketView } from './ticket'

/** The cards each lane shows: every row, or the ones that pass the filters (one pass per change, shared). */
const shownByLane = createRoot(() =>
  createMemo(() => {
    const f = filters()
    const on = filtering()
    const ctx = matchContext()
    return new Map(lanes().map((l) => [l.id, on ? l.rows.filter((row) => matches(row, f, ctx)) : l.rows]))
  }),
)
const shownRows = (l: Lane): Summary[] => shownByLane().get(l.id) ?? l.rows

export function RootPage() {
  const [ready, setReady] = createSignal(false)
  let menuButton: NodeRef | undefined

  onMount(() => {
    void restore().finally(() => setReady(true))
  })

  // Live only while some page of the app is on screen: bind, then read; visible again binds and reads again.
  createEffect(() => {
    if (!ready() || !iii.env.visible) return
    onCleanup(goLive())
    void loadAgents()
    setNow(Date.now())
    const tick = setInterval(() => setNow(Date.now()), 60_000)
    onCleanup(() => {
      clearInterval(tick)
      stampVisit()
    })
  })

  // Back online: what the bus said meanwhile is lost; read again.
  createEffect(
    on(
      () => iii.env.online,
      (now, before) => {
        if (now && before === false && iii.env.visible && ready()) void load(true)
      },
    ),
  )

  const total = () => lanes().reduce((sum, l) => sum + l.total, 0)
  const subtitle = () => {
    if (!meta()) return null
    if (!filtering()) return `${count(total())} ${total() === 1 ? 'ticket' : 'tickets'}`
    const shown = lanes().reduce((sum, l) => sum + shownRows(l).length, 0)
    return `${shown} of ${count(total())} tickets`
  }

  const newTicket = async () => {
    const m = meta()
    if (!m) return
    // A phone shows one lane: the new ticket starts there. A wide board starts it where the board says.
    const status = iii.env.windowClass === 'compact' && lane() ? (lane() as string) : m.defaultStatus
    const result = await iii.nav.push<{ key: string; status: string }>('new', { status })
    if (result && typeof result.key === 'string') openCard({ key: result.key, status: String(result.status ?? status) })
  }

  const boardMenu = async () => {
    const items: [string, string, () => unknown][] = [
      ['Releases', 'rocket', () => iii.nav.push('releases')],
      ['Projects', 'folder', () => iii.nav.push('projects')],
      ['Refresh', 'refresh-cw', () => load()],
    ]
    if (filtering()) items.push(['Clear filters', 'x', () => setFilters(EMPTY_FILTERS)])
    const choice = await iii.ui.actionMenu(menuButton ?? null, {
      actions: items.map(([label, icon]) => ({ label, icon })),
    })
    await items[choice]?.[2]()
  }

  return (
    <Page
      testTag="board"
      title="Kanban"
      subtitle={subtitle()}
      actions={[
        <IconButton
          testTag="filters"
          icon="funnel"
          label={filtering() ? `Filters (${activeCount(filters())})` : 'Filters'}
          active={filtering()}
          enabled={!!meta()}
          onPress={() => void iii.nav.push('filters')}
        />,
        <IconButton
          testTag="new-ticket"
          icon="plus"
          label="New ticket"
          enabled={online() && !!meta()}
          onPress={() => void newTicket()}
        />,
        <IconButton
          testTag="board-menu"
          ref={(node) => {
            menuButton = node
          }}
          icon="ellipsis-vertical"
          label="More"
          onPress={() => void boardMenu()}
        />,
      ]}
      top={
        <Show when={loadError()}>
          {(message: () => string) => (
            <StatusPanel
              testTag="board-error"
              variant="alert"
              headline="Couldn't read the board"
              detail={message()}
              padding={[8, 12]}
              action={
                <Button testTag="board-retry" small enabled={online()} onPress={() => void load()}>
                  Retry
                </Button>
              }
            />
          )}
        </Show>
      }
    >
      <Show
        when={meta()}
        fallback={
          <Show when={!loadError()}>
            <Spinner testTag="board-loading" padding={32} />
          </Show>
        }
      >
        <Show
          when={meta()?.columns.length}
          fallback={
            <EmptyState
              testTag="no-columns"
              icon="columns-2"
              title="No columns configured"
              description="Add one in the Kanban settings of the console."
            />
          }
        >
          <ListDetail
            testTag="board-split"
            listTitle="Board"
            listFill
            detailFullHeight
            showDetail={selected() !== null}
            onDismiss={() => setSelected(null)}
            list={<Board onNew={() => void newTicket()} />}
            detail={
              <Show when={selected()} keyed>
                {(key: string) => <TicketView ticketKey={key} />}
              </Show>
            }
          />
        </Show>
      </Show>
    </Page>
  )
}

/** The lanes: one List of cards per column, all in one drag group. */
function Board(props: { onNew: () => void }) {
  const columns = createMemo(() => meta()?.columns ?? [], undefined, {
    equals: (a, b) => a.length === b.length && a.every((c, i) => c.id === b[i]?.id && c.label === b[i]?.label),
  })
  const sources = new Map<string, ReturnType<typeof createList<CardRow>>>()
  const sourceOf = (id: string) => {
    let source = sources.get(id)
    if (!source) {
      source = createList<CardRow>({ key: 'key' })
      sources.set(id, source)
    }
    return source
  }
  /** List node → its lane: the source of a drop. */
  const listLane = new Map<NodeRef, string>()

  const index = () => {
    const at = columns().findIndex((c) => c.id === lane())
    return at < 0 ? 0 : at
  }
  // A lane that is gone (the configuration changed) falls back to the first.
  createEffect(() => {
    const cols = columns()
    if (cols.length && !cols.some((c) => c.id === lane())) setLane(cols[0].id)
  })

  // What every card shares; a new object only when one of these changes (then every row is computed again).
  const highlight = createMemo(() => filters().labels, undefined, {
    equals: (a, b) => a.length === b.length && a.every((l, i) => l === b[i]),
  })
  const base = createMemo(() => {
    const m = meta()
    return {
      match: matchContext(),
      agents: new Map(agents().map((a) => [a.id, a])),
      highlight: highlight(),
      priorities: m?.priorities ?? [],
      done: m?.done ?? '',
    }
  })
  // What a card looks up on the board: its parent's key, its subtasks (only on a whole board: a part would count short).
  const lookups = createMemo(() => {
    const m = meta()
    const rows = allRows(lanes())
    return {
      keys: new Map(rows.map((r) => [r.id, r.key])),
      subs: complete() ? subtaskStats(rows, m?.done ?? '', m?.canceled ?? null) : null,
    }
  })
  const rowOf = createCardCache()
  const feed = createFeeder()

  // The cards of each lane follow the lanes, the filters and the context; only what changed travels.
  createEffect(() => {
    const shared = base()
    const ctx = { ...shared, ...lookups(), selected: selected() }
    for (const l of lanes())
      feed(
        sourceOf(l.id),
        shownRows(l).map((row) => rowOf(row, ctx, shared)),
      )
  })

  const counts = () =>
    columns().map((c) => {
      const l = lanes().find((x) => x.id === c.id)
      if (!l) return ''
      return filtering() && complete() && l.total > 0 ? `${shownRows(l).length}/${count(l.total)}` : count(l.total)
    })

  const drop = (to: string) => (key: string | number, at: number, source: NodeRef | null) => {
    const id = String(key)
    const target = lanes().find((l) => l.id === to)
    const from = source ? listLane.get(source) : undefined
    const row = target && allRows(lanes()).find((r) => r.id === id)
    if (!target || !row || from === undefined) return
    if (!online()) {
      toast('The engine is not connected', 'error')
      return
    }
    const visible = sourceOf(to)
      .items()
      .map((item) => item.key)
    const where = dropIndex(target.rows, visible, Number.isFinite(at) ? Math.max(0, Math.floor(at)) : 0, id)
    // Dropped where it already is: nothing to write.
    if (row.status === to && target.rows.findIndex((r) => r.id === id) === where) return
    void moveTicket(id, to, where)
  }

  const open = (item: CardRow) => openCard({ key: item.ticketKey, status: item.status })

  return (
    <Lanes
      testTag="lanes"
      titles={columns().map((c) => c.label)}
      counts={counts()}
      selected={index()}
      onSelect={(i: number) => {
        const c = columns()[i]
        if (c) setLane(c.id)
      }}
    >
      <For each={columns()}>
        {(column) => {
          const laneState = () => lanes().find((l) => l.id === column.id)
          const [pulling, setPulling] = createSignal(false)
          return (
            <List
              testTag={`lane:${column.id}`}
              ref={(node) => {
                listLane.set(node, column.id)
              }}
              weight={1}
              source={sourceOf(column.id)}
              dragGroup="board"
              gap={8}
              contentPadding={[8, 12]}
              refreshing={pulling()}
              onRefresh={() => {
                setPulling(true)
                void load(true).finally(() => setPulling(false))
              }}
              onEndReached={() => {
                if (!filtering()) void more(column.id)
              }}
              onDrop={drop(column.id)}
              templates={[
                (row) => (
                  <Card selected={row.selected} gap={6} onPress={open} onLongPress={(item: CardRow) => cardMenu(item)}>
                    <Row gap={4} visible={row.hasParent}>
                      <Icon name="corner-down-right" size={14} tone="faint" />
                      <Text role="small" tone="muted" text={row.parent} />
                    </Row>
                    <Text role="strong" maxLines={3} text={row.title} />
                    <Row gap={4} wrap visible={row.hasChips}>
                      <Chip text={row.l0} selected={row.sel0} visible={row.hasL0} />
                      <Chip text={row.l1} selected={row.sel1} visible={row.hasL1} />
                      <Chip text={row.l2} selected={row.sel2} visible={row.hasL2} />
                      <Chip text={row.more} visible={row.hasMore} />
                      <Chip text={row.p0} icon={row.p0Icon} visible={row.hasP0} />
                      <Chip text={row.p1} icon={row.p1Icon} visible={row.hasP1} />
                    </Row>
                    <Row gap={6}>
                      <Text role="idSmall" tone="muted" text={row.ticketKey} />
                      <Badge text={row.priority} variant={row.priorityVariant} visible={row.hasPriority} />
                      <Row weight={1} />
                      <Row gap={4} visible={row.changed}>
                        <StatusDot tone="accent" />
                        <Text role="small" tone="muted" text={row.changedAt} />
                      </Row>
                      <Row gap={2} visible={row.hasSubs}>
                        <Icon name="list-tree" size={14} />
                        <Text role="small" tone="muted" text={row.subs} />
                        <StatusDot tone="warn" visible={row.subsWarn} />
                      </Row>
                      <Row gap={2} visible={row.hasComments}>
                        <Icon name="message-circle" size={14} />
                        <Text role="small" tone="muted" text={row.comments} />
                      </Row>
                      <Icon name={row.assigneeIcon} hue={row.assigneeHue} size={16} />
                    </Row>
                  </Card>
                ),
              ]}
              empty={
                <Show when={loaded() && laneState()} fallback={<Spinner padding={24} />}>
                  <Show
                    when={filtering()}
                    fallback={
                      <EmptyState
                        icon="columns-2"
                        title={`No tickets in ${column.label}`}
                        actions={
                          <Button small icon="plus" enabled={online()} onPress={props.onNew}>
                            New ticket
                          </Button>
                        }
                      />
                    }
                  >
                    <EmptyState
                      icon="funnel"
                      title="No ticket here matches the filters"
                      actions={
                        <Button small onPress={() => setFilters(EMPTY_FILTERS)}>
                          Clear filters
                        </Button>
                      }
                    />
                  </Show>
                </Show>
              }
              footer={
                <Show when={!filtering() && laneState() && partial(laneState() as Lane)}>
                  <Column gap={6} padding={12} align="center">
                    <Text role="small" tone="muted">
                      {`Showing ${laneState()?.rows.length ?? 0} of ${laneState()?.total ?? 0}`}
                    </Text>
                    <Button
                      small
                      testTag={`more:${column.id}`}
                      loading={paging().has(column.id)}
                      enabled={online()}
                      onPress={() => void more(column.id)}
                    >
                      Load more
                    </Button>
                  </Column>
                </Show>
              }
            />
          )
        }}
      </For>
    </Lanes>
  )
}

/** The long press on a card (or a lift released without moving, D7). */
async function cardMenu(item: CardRow): Promise<void> {
  const m = meta()
  if (!m) return
  const choice = await iii.ui.actionMenu(null, {
    title: item.ticketKey,
    subtitle: item.title,
    actions: [
      { label: 'Open', icon: 'arrow-right' },
      { label: 'Move to…', icon: 'columns-2' },
      { label: 'Assign…', icon: 'bot' },
      { label: 'Priority…', icon: 'flag' },
      { label: 'Copy key', icon: 'copy' },
      { label: 'Share', icon: 'share-2' },
    ],
  })
  const row = allRows(lanes()).find((r) => r.id === item.key)
  if (choice < 0 || !row) return
  try {
    if (choice === 0) openCard(row)
    else if (choice === 1) {
      const to = await iii.ui.actionMenu(null, {
        title: `Move ${row.key} to`,
        actions: m.columns.map((c) => ({ label: c.label, checked: c.id === row.status })),
      })
      const column = m.columns[to]
      if (column && column.id !== row.status) await moveTicket(row.id, column.id)
    } else if (choice === 2) await assign(row)
    else if (choice === 3) await setPriority(row)
    else if (choice === 4) {
      await iii.clipboard.writeText(row.key)
      toast(`Copied ${row.key}`)
    } else if (choice === 5) await iii.share({ text: shareText(row.key, row.title), title: row.key })
  } catch (err) {
    toast(readable(err), 'error')
  }
}

async function assign(row: Summary): Promise<void> {
  const picked = await pickAgent(row.assignee)
  if (picked === undefined || picked === row.assignee) return
  if (!online()) return toast('The engine is not connected', 'error')
  wrote(await kanban.update(row.id, { assignee: picked }))
  toast(picked ? `Assigned ${row.key} to ${agentOf(picked)?.name ?? picked}` : `Unassigned ${row.key}`)
}

async function setPriority(row: Summary): Promise<void> {
  const priorities = meta()?.priorities ?? []
  const choice = await iii.ui.actionMenu(null, {
    title: `Priority of ${row.key}`,
    actions: priorities.map((p) => ({ label: priorityLabel(p), checked: p === row.priority })),
  })
  const priority = priorities[choice]
  if (!priority || priority === row.priority) return
  if (!online()) return toast('The engine is not connected', 'error')
  wrote(await kanban.update(row.id, { priority }))
}
