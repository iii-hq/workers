import { type Host, type PageRenderProps, ShadcnDialog, ShadcnDialogContent, useConfirm } from '@iii-dev/console-ui'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { type KeyboardEvent, useCallback, useRef } from 'react'
import type { AgentProfile, Column } from './shared'
import { TicketScreen } from './ticket'
import { IconAction } from './ui/icon-action'

/** Keys typed into a field are text, never navigation. */
function typing(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el) return false
  return (
    el.isContentEditable ||
    /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) ||
    Boolean(el.closest('[role="menu"], [role="listbox"]'))
  )
}

/**
 * A ticket over the board, Linear's way: the board stays where it was (its scroll, filters and swimlanes are
 * never unmounted), the ticket screen fills a large dialog, and closing it is Escape, the ✕ or a click outside.
 * A linked ticket opened from inside replaces the one shown; "Open in its own pane" still moves it to a pane.
 * Previous / next (the head's arrows, or K / J) walk the board's tickets in the order they are drawn — the project
 * and filters on screen — without closing anything. Leaving a ticket with an unsaved title, description or comment
 * (another ticket, or closing) asks first, as closing its pane does.
 */
export function TicketDialog({
  host,
  ticketId,
  onOpenTicket,
  onClose,
  columns,
  priorities,
  agents,
  doneStatus,
  canceledStatus,
  commands,
  order = [],
}: {
  host: Host
  /** The key or id on screen; null = closed. */
  ticketId: string | null
  onOpenTicket: (key: string) => void
  onClose: () => void
  columns: Column[]
  priorities: string[]
  agents: AgentProfile[]
  doneStatus: string
  canceledStatus: string | null
  commands?: PageRenderProps['commands']
  /** The tickets on screen, in the order they are drawn: what previous / next walk. */
  order?: readonly { id: string; key: string }[]
}) {
  // The dialog opens on its content, not its first button: focus there would show that button's tooltip, and the
  // first Escape would close the tooltip instead of the dialog.
  const start = useRef<HTMLDivElement | null>(null)
  // What the screen reports through `setDirty`, as a pane's does to the host. A new ticket's screen reports afresh.
  const dirty = useRef(false)
  const setDirty = useCallback((next: boolean | string) => {
    dirty.current = Boolean(next)
  }, [])
  const { confirm, dialog: confirmDialog } = useConfirm()
  /** Every way off the ticket on screen goes through here: nothing typed is dropped without asking. */
  const leave = async (go: () => void) => {
    if (
      dirty.current &&
      !(await confirm({
        title: 'Discard your changes?',
        description: "What you typed in this ticket hasn't been saved.",
        confirmLabel: 'Discard',
        cancelLabel: 'Keep editing',
        tone: 'danger',
      }))
    )
      return
    dirty.current = false
    go()
  }
  const openTicket = (key: string) => void leave(() => onOpenTicket(key))
  const close = () => void leave(onClose)
  const at = ticketId ? order.findIndex((t) => t.key === ticketId || t.id === ticketId) : -1
  const prev = at > 0 ? order[at - 1] : null
  const next = at >= 0 && at < order.length - 1 ? order[at + 1] : null
  const nav =
    at >= 0 && order.length > 1 ? (
      <span className="kb-ticket-nav">
        <span aria-live="polite" className="kb-ticket-nav__pos">
          {at + 1} / {order.length}
        </span>
        <IconAction disabled={!prev} label="Previous ticket (K)" onClick={() => prev && openTicket(prev.key)}>
          <ChevronUp size={16} />
        </IconAction>
        <IconAction disabled={!next} label="Next ticket (J)" onClick={() => next && openTicket(next.key)}>
          <ChevronDown size={16} />
        </IconAction>
      </span>
    ) : null
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.metaKey || event.ctrlKey || event.altKey || typing(event.target)) return
    const key = event.key.toLowerCase()
    const to = key === 'j' ? next : key === 'k' ? prev : null
    if (!to) return
    event.preventDefault()
    openTicket(to.key)
  }
  return (
    <ShadcnDialog
      onOpenChange={(open: boolean) => {
        if (!open) close()
      }}
      open={ticketId !== null}
    >
      <ShadcnDialogContent
        className="kb-dialog kb-ticket-dialog"
        initialFocus={start}
        onKeyDown={onKeyDown}
        showCloseButton={false}
      >
        <div className="kb-ticket-dialog__body" ref={start} tabIndex={-1}>
          {ticketId ? (
            <TicketScreen
              agents={agents}
              canceledStatus={canceledStatus}
              columns={columns}
              commands={commands}
              doneStatus={doneStatus}
              host={host}
              // One screen per ticket: drafts, reply target and errors never travel to the next one.
              key={ticketId}
              onBack={close}
              onOpenTicket={openTicket}
              priorities={priorities}
              dialogNav={nav}
              setDirty={setDirty}
              ticketId={ticketId}
              variant="dialog"
            />
          ) : null}
        </div>
        {confirmDialog}
      </ShadcnDialogContent>
    </ShadcnDialog>
  )
}
