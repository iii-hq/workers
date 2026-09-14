import type { Host } from '@iii-dev/console-ui'
import { BoardPage } from './src/board'
import { KanbanConfigForm } from './src/overlays'
import { createKanbanTriggerRenderer, createTicketRenderer } from './src/renderers'
import { BOARD_PAGE_ID, COLUMNS_CONFIGURATION_ID, TICKET_PAGE_ID } from './src/shared'
import { TicketPage } from './src/ticket'

export default function setup(host: Host) {
  host.pages.register({
    id: BOARD_PAGE_ID,
    title: 'Kanban',
    configurationId: COLUMNS_CONFIGURATION_ID,
    render: (props) => <BoardPage {...props} host={host} />,
  })

  host.pages.register({
    id: TICKET_PAGE_ID,
    title: 'Ticket',
    render: (props) => <TicketPage {...props} host={host} />,
  })

  host.configForms.register(COLUMNS_CONFIGURATION_ID, KanbanConfigForm)
  host.functionTriggers.register(createTicketRenderer(host))
  host.triggerRenderers?.register(createKanbanTriggerRenderer(host))
}
