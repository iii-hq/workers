import { ShadcnCard } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { ArrowUpRight, ChevronRight, ListTree, MessageSquare } from 'lucide-react'
import { type DragEvent, memo } from 'react'
import { type AgentProfile, type Column, Pills, type TicketSummary } from '../shared'
import { AssigneeAvatar } from '../ui/avatar'
import { PriorityIcon } from '../ui/glyphs'
import { IconAction } from '../ui/icon-action'
import { LabelList } from '../ui/label-badge'
import { StatusMenu } from '../ui/properties'

export type TicketCardProps = {
  ticket: TicketSummary
  dragging: boolean
  changed: boolean
  now: number
  agents: AgentProfile[]
  /** The parent's key, when the board holds the parent. */
  parentKey: string | undefined
  /** The parent's title, shown truncated after its key (Linear's `KEY › parent`). */
  parentTitle?: string
  subs: { done: number; total: number } | undefined
  /** The labels the filters ask for: they come first and show selected. */
  highlight: string[]
  priorities: string[]
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  canOpenPane: boolean
  /** A click on the card: the ticket in this pane. */
  onOpen: (key: string) => void
  /** The corner action: the ticket in a pane of its own. */
  onOpenPane: (key: string) => void
  onStatus: (ticket: TicketSummary, status: string) => void
  onDragStart: (ticket: TicketSummary, event: DragEvent<HTMLElement>) => void
}

/* Tailwind (prefixed `tw:`, compiled and scoped by tailwind.mjs). Dark: a black card with a neutral-800
   border on a neutral-900 lane, neutral-700 on hover. Light: white on neutral-100 with neutral-200. */
const CARD = [
  'tw:group tw:relative tw:gap-1.5 tw:rounded-xl tw:px-3.5 tw:py-3 tw:text-[13px] tw:select-none tw:cursor-default',
  'tw:border tw:border-neutral-100 tw:bg-white tw:text-neutral-900 tw:ring-0 tw:shadow-none',
  'tw:hover:border-neutral-200',
  'tw:dark:border-neutral-900 tw:dark:bg-neutral-950 tw:dark:text-neutral-100 tw:dark:hover:border-neutral-800',
  'tw:focus-visible:outline-2 tw:focus-visible:outline-offset-1 tw:focus-visible:outline-neutral-400',
  'tw:data-[dragging=true]:opacity-40',
].join(' ')
const MUTED = 'tw:text-neutral-500 tw:dark:text-neutral-400'
const KEY = `tw:shrink-0 tw:text-[13px] tw:font-medium tw:tabular-nums ${MUTED}`
const TITLE =
  'tw:m-0 tw:min-w-0 tw:flex-1 tw:line-clamp-2 tw:text-sm tw:leading-5 tw:font-medium tw:tracking-[-0.006em] tw:wrap-anywhere tw:text-neutral-900 tw:dark:text-neutral-100'
/** An outlined chip in the meta row (priority, subtasks, comments). */
const CHIP =
  'tw:inline-flex tw:h-6 tw:items-center tw:gap-1.5 tw:rounded-full tw:border tw:border-neutral-100 tw:text-[13px] tw:font-medium tw:tabular-nums tw:dark:border-neutral-800 tw:[&>svg]:size-[14px]'

/**
 * One card, three lines: key and assignee; the status picker and the title; then priority, labels, pills and the
 * counts. Memoised: a drag over the board redraws the cells it crosses, not every card around them.
 */
export const TicketCard = memo(function TicketCard({
  ticket,
  dragging,
  changed,
  now,
  agents,
  parentKey,
  parentTitle,
  subs,
  highlight,
  priorities,
  columns,
  doneStatus,
  canceledStatus,
  canOpenPane,
  onOpen,
  onOpenPane,
  onStatus,
  onDragStart,
}: TicketCardProps) {
  // A parent in the done status with subtasks still open says so.
  const openSubs = subs && ticket.status === doneStatus ? subs.total - subs.done : 0
  const subsLabel = subs
    ? `${subs.done} of ${subs.total} subtasks done${openSubs > 0 ? `; done with ${openSubs} open` : ''}`
    : ''
  const hasMeta = ticket.labels.length > 0 || (ticket.pills?.length ?? 0) > 0 || subs || ticket.comment_count > 0
  return (
    <ShadcnCard
      aria-label={[
        `${ticket.key}: ${ticket.title}`,
        parentKey ? `subtask of ${parentKey}` : '',
        subsLabel,
        changed ? 'changed since your last visit' : '',
      ]
        .filter(Boolean)
        .join(', ')}
      className={`kb-card ${CARD}`}
      data-changed={changed ? 'true' : undefined}
      data-dragging={dragging ? 'true' : undefined}
      data-ticket-card=""
      data-ticket-id={ticket.id}
      draggable
      onClick={() => onOpen(ticket.key)}
      onDragStart={(event) => onDragStart(ticket, event)}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onOpen(ticket.key)
        }
      }}
      role="button"
      size="sm"
      tabIndex={0}
    >
      <div className="tw:flex tw:min-w-0 tw:items-center tw:gap-1.5">
        <span className={KEY}>{ticket.key}</span>
        {parentKey ? (
          <span
            className={`tw:flex tw:min-w-0 tw:items-center tw:gap-1 ${MUTED}`}
            title={`Subtask of ${parentKey}${parentTitle ? ` — ${parentTitle}` : ''}`}
          >
            <ChevronRight className="tw:shrink-0 tw:size-[14px]" size={16} />
            <span className="tw:truncate">{parentTitle ?? parentKey}</span>
          </span>
        ) : null}
        {changed ? (
          <span
            className="tw:flex tw:shrink-0 tw:items-center tw:gap-1 tw:text-xs tw:font-medium tw:text-glyph-blue"
            title={`Changed ${formatRelative(ticket.updated_at, now)} — since your last visit`}
          >
            <span aria-hidden="true" className="tw:size-1.5 tw:rounded-full tw:bg-glyph-blue" />
            {formatRelative(ticket.updated_at, now)}
          </span>
        ) : null}
        <span className="tw:flex-1" />
        {canOpenPane ? (
          <span className="tw:flex tw:opacity-0 tw:transition-opacity tw:group-hover:opacity-100 tw:group-focus-within:opacity-100 tw:pointer-coarse:opacity-100">
            <IconAction
              label="Open in a pane"
              onClick={(event) => {
                event.stopPropagation()
                onOpenPane(ticket.key)
              }}
              size="icon-xs"
            >
              <ArrowUpRight size={16} />
            </IconAction>
          </span>
        ) : null}
        <AssigneeAvatar agents={agents} id={ticket.assignee} />
      </div>
      <div className="tw:flex tw:items-start tw:gap-2">
        <StatusMenu
          canceledStatus={canceledStatus}
          columns={columns}
          doneStatus={doneStatus}
          iconOnly
          onChange={(status) => onStatus(ticket, status)}
          status={ticket.status}
        />
        <p className={TITLE}>{ticket.title}</p>
      </div>
      <div className="tw:mt-1 tw:flex tw:flex-wrap tw:items-center tw:gap-1.5" data-empty={hasMeta ? undefined : 'true'}>
        <span className={`tw:w-8 tw:justify-center ${CHIP}`} title={`Priority: ${ticket.priority}`}>
          <PriorityIcon priorities={priorities} priority={ticket.priority} />
        </span>
        <LabelList highlight={highlight} labels={ticket.labels} max={2} />
        <Pills pills={ticket.pills} />
        <span className="tw:flex-1" />
        {subs ? (
          <span
            aria-label={subsLabel}
            className={`tw:rounded-full tw:px-2 ${CHIP} ${openSubs > 0 ? 'tw:text-warn' : MUTED}`}
            role="img"
          >
            <ListTree size={16} />
            {subs.done}/{subs.total}
          </span>
        ) : null}
        {ticket.comment_count > 0 ? (
          <span aria-label={`${ticket.comment_count} comments`} className={`tw:rounded-full tw:px-2 ${CHIP} ${MUTED}`} role="img">
            <MessageSquare size={16} />
            {ticket.comment_count}
          </span>
        ) : null}
      </div>
      <p className={`tw:m-0 tw:text-xs ${MUTED}`}>Created {createdDay(ticket.created_at, now)}</p>
    </ShadcnCard>
  )
})

/** `Oct 21`, or `Mar 2025` once it is from another year — Linear's card footer. */
function createdDay(iso: string, now: number): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  const sameYear = date.getFullYear() === new Date(now).getFullYear()
  return date.toLocaleDateString('en-US', sameYear ? { month: 'short', day: 'numeric' } : { month: 'short', year: 'numeric' })
}
