import { type Host, type PageRenderProps, ShadcnDialog, ShadcnDialogContent } from '@iii-dev/console-ui'
import type { AgentProfile, Column } from './shared'
import { TicketScreen } from './ticket'

/**
 * A ticket over the board, Linear's way: the board stays where it was (its scroll, filters and swimlanes are
 * never unmounted), the ticket screen fills a large dialog, and closing it is Escape, the ✕ or a click outside.
 * A linked ticket opened from inside replaces the one shown; "Open in its own pane" still moves it to a pane.
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
}) {
  return (
    <ShadcnDialog
      onOpenChange={(open: boolean) => {
        if (!open) onClose()
      }}
      open={ticketId !== null}
    >
      <ShadcnDialogContent className="kb-dialog kb-ticket-dialog" showCloseButton={false}>
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
            onBack={onClose}
            onOpenTicket={onOpenTicket}
            priorities={priorities}
            ticketId={ticketId}
            variant="dialog"
          />
        ) : null}
      </ShadcnDialogContent>
    </ShadcnDialog>
  )
}
