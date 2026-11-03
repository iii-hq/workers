import { ShadcnButton, ShadcnSkeleton } from '@iii-dev/console-ui'
import { Plus } from 'lucide-react'
import type { DragEvent, ReactNode } from 'react'
import type { BoardColumn as Lane, Column, TicketSummary } from '../shared'
import type { DropTarget } from '../swimlanes'
import { ColumnStatusIcon } from '../ui/glyphs'
import { IconAction } from '../ui/icon-action'

/* Tailwind (`tw:`, see tailwind.mjs). Lanes: neutral-900 on the black board in dark, a soft neutral-200 in light. */
const MUTED = 'tw:text-neutral-500 tw:dark:text-neutral-400'
const COLUMN = [
  'kb-column tw:flex tw:min-h-0 tw:max-w-[360px] tw:flex-[0_0_320px] tw:flex-col tw:rounded-2xl',
  'tw:bg-neutral-200/50 tw:dark:bg-[#0f0f0f]',
  'tw:data-[drop=true]:bg-neutral-200 tw:dark:data-[drop=true]:bg-neutral-800',
].join(' ')

/** The horizontal strip of columns: 24px gutters, 12px between lanes, a thin quiet scrollbar. */
export const COLUMNS =
  'tw:flex tw:min-h-0 tw:flex-1 tw:gap-3 tw:overflow-x-auto tw:overflow-y-hidden tw:px-6 tw:pt-4 tw:pb-6 tw:[scrollbar-width:thin]'

/** A column's heading: status glyph, name, count, and its own add. */
export function ColumnHeader({
  column,
  columns,
  doneStatus,
  canceledStatus,
  count,
  onAdd,
}: {
  column: Column
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  count: string
  onAdd: () => void
}) {
  return (
    <header className="tw:flex tw:h-12 tw:shrink-0 tw:items-center tw:gap-2.5 tw:pr-2 tw:pl-3.5">
      <ColumnStatusIcon canceledStatus={canceledStatus} columns={columns} doneStatus={doneStatus} status={column.id} />
      <h3 className="tw:m-0 tw:truncate tw:text-sm tw:font-medium tw:text-neutral-900 tw:dark:text-neutral-100">{column.label}</h3>
      <span className={`tw:text-sm tw:font-medium tw:tabular-nums ${MUTED}`}>{count}</span>
      <span className="tw:flex-1" />
      <IconAction label={`New ticket in ${column.label}`} onClick={onAdd} size="icon-xs">
        <Plus size={16} />
      </IconAction>
    </header>
  )
}

/** The slot a dragged card would land in. */
export function DropSlot({ height }: { height: number }) {
  return (
    <div
      aria-hidden="true"
      className="tw:shrink-0 tw:rounded-lg tw:border tw:border-dashed tw:border-neutral-300 tw:bg-neutral-200/60 tw:dark:border-neutral-700 tw:dark:bg-black/40"
      style={{ height }}
    />
  )
}

/**
 * One status column: its header (left out in a narrow pane, where tabs name the column), its cards, the drop
 * slot while a card is over it, and an empty state that adds a ticket.
 */
export function BoardColumnView({
  column,
  columns,
  doneStatus,
  canceledStatus,
  cards,
  count,
  head,
  filtering,
  drag,
  dropTarget,
  renderCard,
  onAdd,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  column: Lane
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  /** The cards the filters keep. */
  cards: TicketSummary[]
  count: string
  head: boolean
  filtering: boolean
  drag: { id: string; height: number } | null
  dropTarget: DropTarget | null
  renderCard: (ticket: TicketSummary, dragging: boolean) => ReactNode
  onAdd: () => void
  onDragOver: (event: DragEvent<HTMLElement>) => void
  onDragLeave: (event: DragEvent<HTMLElement>) => void
  onDrop: (event: DragEvent<HTMLElement>) => void
}) {
  const isDropping = dropTarget?.lane === null && dropTarget.status === column.id && drag !== null
  // dropTarget.index counts the visible cards without the dragged one.
  const others = cards.filter((ticket) => ticket.id !== drag?.id)
  const slotHeight = drag?.height ?? 92
  return (
    <section
      aria-label={`${column.label}, ${count} ${column.tickets.length === 1 ? 'ticket' : 'tickets'}`}
      className={COLUMN}
      data-drop={isDropping ? 'true' : undefined}
      data-lane={column.id}
      onDragLeave={onDragLeave}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      {head ? (
        <ColumnHeader
          canceledStatus={canceledStatus}
          column={column}
          columns={columns}
          count={count}
          doneStatus={doneStatus}
          onAdd={onAdd}
        />
      ) : null}
      <div className="kb-column__body tw:flex tw:min-h-0 tw:flex-1 tw:flex-col tw:gap-2 tw:overflow-y-auto tw:overscroll-contain tw:px-2 tw:pt-0.5 tw:pb-3 tw:[scrollbar-width:thin]">
        {cards.length === 0 && !isDropping ? (
          filtering ? (
            <p className={`tw:mx-1 tw:mt-1 tw:text-[13px] ${MUTED}`}>No ticket here matches the filters.</p>
          ) : (
            <ShadcnButton
              className={`tw:h-[72px] tw:w-full tw:justify-center tw:rounded-lg tw:border tw:border-dashed tw:border-neutral-300 tw:text-xs tw:dark:border-neutral-800 ${MUTED}`}
              onClick={onAdd}
              type="button"
              variant="ghost"
            >
              <Plus size={16} />
              Add a ticket
            </ShadcnButton>
          )
        ) : null}
        {cards.map((ticket) => (
          <div className="tw:flex tw:flex-col tw:gap-2" key={ticket.id}>
            {isDropping && dropTarget && others[dropTarget.index]?.id === ticket.id ? <DropSlot height={slotHeight} /> : null}
            {renderCard(ticket, drag?.id === ticket.id)}
          </div>
        ))}
        {isDropping && dropTarget && dropTarget.index >= others.length ? <DropSlot height={slotHeight} /> : null}
      </div>
    </section>
  )
}

/** The board while it is first read: five columns of placeholder cards. */
export function BoardSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading the board" className={`kb-columns ${COLUMNS}`}>
      {[3, 2, 4, 1, 2].map((cards, index) => (
        <section className={COLUMN} key={index}>
          <header className="tw:flex tw:h-12 tw:items-center tw:px-3.5">
            <ShadcnSkeleton className="tw:h-3.5 tw:w-24" />
          </header>
          <div className="tw:flex tw:flex-col tw:gap-2.5 tw:px-2">
            {Array.from({ length: cards }, (_, card) => (
              <ShadcnSkeleton className="tw:h-[92px] tw:w-full tw:rounded-lg" key={card} />
            ))}
          </div>
        </section>
      ))}
    </div>
  )
}
