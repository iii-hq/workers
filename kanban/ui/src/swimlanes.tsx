import { IconButton, Kbd, uiClasses } from '@iii-dev/console-ui'
import { Archive, Ban, ChevronDown, ChevronRight, Flag, Folder, Inbox, ListTree, Move, Plus } from 'lucide-react'
import { type CSSProperties, type DragEvent, memo, type ReactNode, useLayoutEffect, useRef } from 'react'
import { cellWindow, type Swimlane, shortDay } from './board-state'
import { type BoardColumn, StatusGlyph, type TicketSummary } from './shared'

/** Where a dragged card would land: a cell (`lane` is null on a board without swimlanes) and a position in it. */
export type DropTarget = {
  lane: string | null
  status: string
  /** Among the cards drawn in the cell, the dragged one left out. */
  index: number
  /** Ids of those cards, in order. */
  ids: string[]
  /** Where the dragged card sits among the cards drawn in the cell, -1 when it is not in it. */
  at: number
}

export type Dragging = { id: string; height: number }

const STATE_LABEL = { planned: 'Planned', active: 'Active', released: 'Released', archived: 'Archived' } as const
const STATE_TONE = { planned: undefined, active: 'ink', released: 'ok', archived: 'faint' } as const

const plural = (n: number) => `${n} ${n === 1 ? 'ticket' : 'tickets'}`

/** The key of a cell in `expanded`. */
export const cellKey = (laneId: string, status: string) => `${laneId}|${status}`

/** Everything the wide grid and the narrow sections share. */
type CommonProps = {
  lanes: Swimlane[]
  columns: BoardColumn[]
  collapsed: (lane: Swimlane) => boolean
  onToggle: (lane: Swimlane) => void
  /** Cells opened up in place (`laneId|status`). */
  expanded: ReadonlySet<string>
  onExpand: (cell: string) => void
  drag: Dragging | null
  dropTarget: DropTarget | null
  /** Why the card in the air cannot be dropped in this swimlane; null when it can, or when no card is in the air. */
  refuseFor: (lane: Swimlane) => string | null
  /** Must keep its identity while a card is dragged, or every cell is redrawn at each move. */
  renderCard: (ticket: TicketSummary, dragging: boolean) => ReactNode
  onDragOver: (lane: Swimlane, status: string, event: DragEvent<HTMLElement>) => void
  onDragLeave: (event: DragEvent<HTMLElement>) => void
  onDrop: (lane: Swimlane, status: string, event: DragEvent<HTMLElement>) => void
  onAdd: (lane: Swimlane, column: BoardColumn) => void
  /** Open the release, project or parent a swimlane stands for; absent when the console cannot open panes. */
  onOpenLane?: (lane: Swimlane) => void
  statusLabel: (status: string) => string
  now: number
  /** Said when there is no swimlane to draw (a filter that matches nothing). */
  empty: string
}

function LaneGlyph({ lane }: { lane: Swimlane }) {
  if (lane.kind === 'group') return <Archive size={16} />
  if (lane.kind === 'none') return <Inbox size={16} />
  if (lane.facet === 'release') return <Flag size={16} />
  if (lane.facet === 'parent') return <ListTree size={16} />
  return (
    <span className={uiClasses.treeItemIcon} data-color={lane.color ?? undefined}>
      <Folder size={16} />
    </span>
  )
}

function LaneName({ lane, onOpenLane }: { lane: Swimlane; onOpenLane?: (lane: Swimlane) => void }) {
  const text =
    lane.facet === 'parent' && lane.kind === 'entity' ? (
      <>
        <span className="kanban-card__key">{lane.short}</span>
        <span className="kanban-swim__title">{lane.name}</span>
      </>
    ) : (
      lane.name
    )
  if (lane.kind !== 'entity' || !onOpenLane) return <span className="kanban-swim__name">{text}</span>
  return (
    <button className="kanban-swim__name" data-link="true" onClick={() => onOpenLane(lane)} type="button">
      {text}
    </button>
  )
}

/** State, target, progress: what a swimlane says about itself next to its name. */
function LaneMeta({
  lane,
  now,
  statusLabel,
}: {
  lane: Swimlane
  now: number
  statusLabel: (status: string) => string
}) {
  if (lane.kind === 'none') return <span className="kanban-swim__prog">{plural(lane.count)}</span>
  if (lane.kind === 'group') {
    return (
      <>
        <span className="kanban-swim__meta">{lane.lanes.map((l) => l.short).join(', ')}</span>
        <span className="kanban-swim__prog">{plural(lane.count)}</span>
      </>
    )
  }
  const unit = lane.facet === 'parent' ? 'subtasks' : 'tickets'
  return (
    <>
      {lane.state ? (
        <span className="kanban-swim__state">
          <span className="kanban-swim__dot" data-tone={STATE_TONE[lane.state]} />
          {STATE_LABEL[lane.state]}
        </span>
      ) : null}
      {lane.facet === 'release' && lane.target ? (
        <span className="kanban-swim__meta">Target {shortDay(lane.target, now)}</span>
      ) : null}
      {lane.facet === 'parent' && lane.parent ? (
        <span className="kanban-swim__meta">{statusLabel(lane.parent.status)}</span>
      ) : null}
      {lane.total > 0 ? (
        <>
          <span aria-label={`${lane.done} of ${lane.total} ${unit} done`} className="kanban-swim__bar" role="img">
            <span style={{ width: `${Math.round((lane.done / lane.total) * 100)}%` }} />
          </span>
          <span className="kanban-swim__prog">
            {lane.done} of {lane.total} done
          </span>
        </>
      ) : (
        // total leaves canceled tickets out: a lane with cards but no total holds only canceled ones.
        <span className="kanban-swim__prog">{lane.count > 0 ? 'All canceled' : 'No tickets'}</span>
      )}
    </>
  )
}

/** The message a refused drop shows on the swimlane while a card is in the air. */
function refusal(reason: string | null, collapsed: boolean, dragging: boolean): string | null {
  return reason ?? (dragging && collapsed ? 'Expand to drop here' : null)
}

/** The drop target when it lands in `lane` (or in a released release the group holds); a drag over other swimlanes does not touch it. */
const dropIn = (target: DropTarget | null, lane: Swimlane) =>
  target && (target.lane === lane.id || lane.lanes.some((member) => member.id === target.lane)) ? target : null

/** What a cell takes from the board, and no more: the memoised cells a drag does not cross compare equal and are not redrawn. */
function cellProps(
  p: Pick<
    CommonProps,
    'drag' | 'dropTarget' | 'expanded' | 'onAdd' | 'onDragLeave' | 'onDragOver' | 'onDrop' | 'onExpand' | 'renderCard'
  >,
  lane: Swimlane,
  column: BoardColumn,
  refused: boolean,
) {
  return {
    column,
    drag: p.drag,
    dropTarget: p.dropTarget?.status === column.id ? p.dropTarget : null,
    lane,
    onAdd: p.onAdd,
    onDragLeave: p.onDragLeave,
    onDragOver: p.onDragOver,
    onDrop: p.onDrop,
    onExpand: p.onExpand,
    open: p.expanded.has(cellKey(lane.id, column.id)),
    refused,
    renderCard: p.renderCard,
  }
}

/**
 * One status of one swimlane: the cards drawn (the first few, the rest behind
 * "Show N more"), the landing slot while a card is over it, an add button
 * when nothing is in it. A refused swimlane takes no drag events, so the
 * browser shows the not-allowed cursor.
 */
const Cell = memo(function Cell({
  lane,
  column,
  narrow,
  open,
  refused,
  drag,
  dropTarget,
  renderCard,
  onDragOver,
  onDragLeave,
  onDrop,
  onAdd,
  onExpand,
}: Pick<CommonProps, 'drag' | 'renderCard' | 'onDragOver' | 'onDragLeave' | 'onDrop' | 'onAdd' | 'onExpand'> & {
  lane: Swimlane
  column: BoardColumn
  narrow?: boolean
  /** Opened up in place ("Show N more" was used). */
  open: boolean
  /** A card in the air cannot be dropped in this swimlane. */
  refused: boolean
  /** Where the card in the air would land, when it is over this cell. */
  dropTarget: DropTarget | null
}) {
  const tickets = lane.cells.get(column.id) ?? []
  const key = cellKey(lane.id, column.id)
  const view = cellWindow(tickets.length, open)
  const drawn = tickets.slice(0, view.shown)
  const isDropping = drag !== null && dropTarget?.lane === lane.id && dropTarget.status === column.id
  // dropTarget.index counts the drawn cards without the dragged one.
  const others = drawn.filter((ticket) => ticket.id !== drag?.id)
  const slot = <div className="kanban-placeholder" style={{ height: drag?.height ?? 58 }} />
  const root = useRef<HTMLDivElement>(null)
  // "Show N more" removes the button that had focus: the first card it reveals takes it.
  const reveal = useRef<number | null>(null)
  useLayoutEffect(() => {
    if (reveal.current === null || view.hidden > 0) return
    const card = root.current?.querySelectorAll<HTMLElement>('[data-ticket-card]')[reveal.current]
    reveal.current = null
    card?.focus()
  }, [view.hidden])
  return (
    // biome-ignore lint/a11y/useSemanticElements: a fieldset brings its own min-width and layout quirks; this is a flex column that names itself
    <div
      aria-label={`${lane.short}, ${column.label}, ${plural(tickets.length)}`}
      className="kanban-swim__cell"
      data-drop={isDropping ? 'true' : undefined}
      data-narrow={narrow ? 'true' : undefined}
      onDragLeave={refused ? undefined : onDragLeave}
      onDragOver={refused ? undefined : (event) => onDragOver(lane, column.id, event)}
      onDrop={refused ? undefined : (event) => onDrop(lane, column.id, event)}
      ref={root}
      role="group"
    >
      {tickets.length === 0 && !isDropping && !lane.refuse ? (
        <button className="kanban-swim__add" onClick={() => onAdd(lane, column)} type="button">
          <Plus size={16} />
          Add to {lane.short} · {column.label}
        </button>
      ) : null}
      {drawn.map((ticket) => (
        <div key={ticket.id}>
          {isDropping && dropTarget && others[dropTarget.index]?.id === ticket.id ? slot : null}
          {renderCard(ticket, drag?.id === ticket.id)}
        </div>
      ))}
      {isDropping && dropTarget && dropTarget.index >= others.length ? slot : null}
      {view.hidden > 0 ? (
        <button
          aria-expanded="false"
          aria-label={`Show ${view.hidden} more in ${lane.short}, ${column.label}`}
          className="kanban-swim__more"
          onClick={() => {
            reveal.current = view.shown
            onExpand(key)
          }}
          type="button"
        >
          Show {view.hidden} more
        </button>
      ) : null}
    </div>
  )
})

/* ── wide: a column header row, then one row of cells per swimlane ───── */

type GridProps = CommonProps & {
  /** The count under a column heading ("3 of 10" under a filter). */
  columnCount: (column: BoardColumn) => string
  onAddInColumn: (column: BoardColumn) => void
  /** For the column glyphs. */
  doneStatus: string
  canceledStatus: string | null
}

const LaneRows = memo(function LaneRows({
  lane,
  nested,
  ...p
}: Omit<GridProps, 'lanes' | 'columnCount' | 'onAddInColumn' | 'doneStatus' | 'canceledStatus'> & {
  lane: Swimlane
  nested?: boolean
}) {
  const isCollapsed = p.collapsed(lane)
  const reason = p.refuseFor(lane)
  const refuse = refusal(reason, isCollapsed, p.drag !== null)
  return (
    <>
      <section
        aria-label={`${lane.name} swimlane`}
        className="kanban-swim"
        data-nested={nested ? 'true' : undefined}
        data-refuse={refuse ? 'true' : undefined}
      >
        <div className="kanban-swim__head">
          <div className="kanban-swim__id">
            <IconButton
              aria-expanded={!isCollapsed}
              label={`${isCollapsed ? 'Expand' : 'Collapse'} ${lane.name}`}
              onClick={() => p.onToggle(lane)}
              variant="ghost"
            >
              {isCollapsed ? <ChevronRight size={16} /> : <ChevronDown size={16} />}
            </IconButton>
            <span aria-hidden="true" className="kanban-swim__glyph">
              <LaneGlyph lane={lane} />
            </span>
            <LaneName lane={lane} onOpenLane={p.onOpenLane} />
            <LaneMeta lane={lane} now={p.now} statusLabel={p.statusLabel} />
            {refuse ? (
              <span className="kanban-swim__refuse">
                <Ban size={16} />
                {refuse}
              </span>
            ) : null}
          </div>
        </div>
        {lane.kind !== 'group' && !isCollapsed
          ? p.columns.map((column) => <Cell key={column.id} {...cellProps(p, lane, column, reason !== null)} />)
          : null}
      </section>
      {lane.kind === 'group' && !isCollapsed
        ? lane.lanes.map((member) => (
            <LaneRows key={member.id} lane={member} nested {...p} dropTarget={dropIn(p.dropTarget, member)} />
          ))
        : null}
    </>
  )
})

export function SwimlaneGrid({ lanes, columnCount, onAddInColumn, doneStatus, canceledStatus, ...p }: GridProps) {
  return (
    <div className="kanban-swimlanes" style={{ '--kanban-cols': p.columns.length } as CSSProperties}>
      <div className="kanban-swim kanban-swim__cols">
        {p.columns.map((column) => (
          <div className="kanban-swim__col" key={column.id}>
            <StatusGlyph
              canceledStatus={canceledStatus}
              columns={p.columns}
              doneStatus={doneStatus}
              status={column.id}
            />
            <span className="kanban-lane__title">{column.label}</span>
            <span className="kanban-lane__count">{columnCount(column)}</span>
            <IconButton
              className="kanban-lane__add"
              label={`New ticket in ${column.label}`}
              onClick={() => onAddInColumn(column)}
              variant="ghost"
            >
              <Plus size={16} />
            </IconButton>
          </div>
        ))}
      </div>
      {lanes.length === 0 ? <p className="kanban-lane__none">{p.empty}</p> : null}
      {lanes.map((lane) => (
        <LaneRows key={lane.id} lane={lane} {...p} dropTarget={dropIn(p.dropTarget, lane)} />
      ))}
    </div>
  )
}

/* ── narrow: one status at a time, a collapsible section per swimlane ── */

type SectionsProps = CommonProps & {
  /** The status tab on screen. */
  column: BoardColumn
  /** Sections with nothing in this status stay when empty swimlanes are kept. */
  showEmpty: boolean
}

/** Tickets a swimlane has in one status (a group: its releases'). */
function visibleIn(lane: Swimlane, status: string): number {
  return lane.kind === 'group'
    ? lane.lanes.reduce((n, member) => n + visibleIn(member, status), 0)
    : (lane.cells.get(status)?.length ?? 0)
}

const Section = memo(function Section({
  lane,
  nested,
  column,
  showEmpty,
  ...p
}: Omit<SectionsProps, 'lanes'> & { lane: Swimlane; nested?: boolean }) {
  const here = visibleIn(lane, column.id)
  if (here === 0 && !showEmpty) return null
  const isCollapsed = p.collapsed(lane)
  const reason = p.refuseFor(lane)
  const refuse = refusal(reason, isCollapsed, p.drag !== null)
  return (
    <>
      <section
        aria-label={`${lane.name}, ${here} in ${column.label}`}
        className="kanban-sec"
        data-nested={nested ? 'true' : undefined}
        data-refuse={refuse ? 'true' : undefined}
      >
        <h3 className="kanban-sec__h">
          <button
            aria-expanded={!isCollapsed}
            className="kanban-sec__head"
            onClick={() => p.onToggle(lane)}
            type="button"
          >
            {isCollapsed ? <ChevronRight size={16} /> : <ChevronDown size={16} />}
            <span aria-hidden="true" className="kanban-swim__glyph">
              <LaneGlyph lane={lane} />
            </span>
            <span className="kanban-sec__line">
              <span className="kanban-sec__name">{lane.name}</span>
              <span className="kanban-lane__count">{here}</span>
            </span>
            {lane.state ? (
              <span className="kanban-swim__state">
                <span className="kanban-swim__dot" data-tone={STATE_TONE[lane.state]} />
                {STATE_LABEL[lane.state]}
              </span>
            ) : (
              <span />
            )}
            {refuse ? (
              <span className="kanban-sec__sub kanban-swim__refuse">
                <Ban size={16} />
                {refuse}
              </span>
            ) : !isCollapsed ? (
              <span className="kanban-sec__sub">
                <LaneMeta lane={{ ...lane, state: null }} now={p.now} statusLabel={p.statusLabel} />
              </span>
            ) : null}
          </button>
        </h3>
        {lane.kind !== 'group' && !isCollapsed ? (
          <Cell narrow {...cellProps(p, lane, column, reason !== null)} />
        ) : null}
      </section>
      {lane.kind === 'group' && !isCollapsed
        ? lane.lanes.map((member) => (
            <Section
              column={column}
              key={member.id}
              lane={member}
              nested
              showEmpty={showEmpty}
              {...p}
              dropTarget={dropIn(p.dropTarget, member)}
            />
          ))
        : null}
    </>
  )
})

export function SwimlaneSections({ lanes, column, showEmpty, ...p }: SectionsProps) {
  return (
    <div className="kanban-sections">
      {lanes.every((lane) => !visibleIn(lane, column.id) && !showEmpty) ? (
        <p className="kanban-lane__none">{p.empty}</p>
      ) : null}
      {lanes.map((lane) => (
        <Section
          column={column}
          key={lane.id}
          lane={lane}
          showEmpty={showEmpty}
          {...p}
          dropTarget={dropIn(p.dropTarget, lane)}
        />
      ))}
    </div>
  )
}

/** The line above the board while a card is in the air: what dropping here would do. */
export function DragLine({ message }: { message: string }) {
  return (
    <div className="kanban-dragline">
      <div aria-atomic="true" aria-live="polite" className="kanban-dragline__msg" role="status">
        <Move size={16} />
        {message}
      </div>
      <span className="kanban-dragline__hint">
        <Kbd>Esc</Kbd>
        cancels
      </span>
    </div>
  )
}
