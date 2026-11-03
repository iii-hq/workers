import { type Host, ShadcnButton } from '@iii-dev/console-ui'
import { useState } from 'react'
import { openPane } from '../plan-dialogs'
import { type AgentProfile, type BoardData, TICKET_PAGE_ID, type TicketSummary } from '../shared'
import { AssigneeAvatar } from '../ui/avatar'
import { ColumnStatusIcon, PriorityIcon } from '../ui/glyphs'
import { LabelList } from '../ui/label-badge'

/**
 * The tickets of one list as Linear-style rows — priority, key, status, title, labels, assignee — the first few,
 * then the rest on request. Each opens its ticket in a pane.
 */
export function TicketRows({
  host,
  tickets,
  data,
  agents,
  canOpenPane,
  label,
  limit = 8,
}: {
  host: Host
  tickets: TicketSummary[]
  /** The board: its columns, done and canceled ones name each row's glyph. */
  data: BoardData
  agents: AgentProfile[]
  canOpenPane: boolean
  label: string
  limit?: number
}) {
  const [all, setAll] = useState(false)
  const shown = all ? tickets : tickets.slice(0, limit)
  return (
    <div className="kb-plan-trows">
      <ul aria-label={label} className="kb-plan-trows__list">
        {shown.map((ticket) => (
          <li key={ticket.id}>
            <button
              className="kb-plan-trow"
              disabled={!canOpenPane}
              onClick={() => openPane(host, TICKET_PAGE_ID, { id: ticket.key })}
              type="button"
            >
              <PriorityIcon priorities={data.priorities} priority={ticket.priority} />
              <span className="kb-plan-trow__key">{ticket.key}</span>
              <ColumnStatusIcon
                canceledStatus={data.canceled_status}
                columns={data.columns}
                doneStatus={data.done_status}
                status={ticket.status}
              />
              <span className="kb-plan-trow__title">{ticket.title}</span>
              <span className="kb-plan-trow__labels">
                <LabelList labels={ticket.labels} max={2} />
              </span>
              <AssigneeAvatar agents={agents} id={ticket.assignee} />
            </button>
          </li>
        ))}
      </ul>
      {tickets.length > shown.length ? (
        <ShadcnButton className="kb-plan-trows__more" onClick={() => setAll(true)} size="sm" type="button" variant="ghost">
          Show {tickets.length - shown.length} more
        </ShadcnButton>
      ) : null}
    </div>
  )
}
