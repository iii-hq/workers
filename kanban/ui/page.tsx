import type { Host, PageCommand } from '@iii-dev/console-ui'
import { BoardPage } from './src/board'
import { KanbanConfigForm } from './src/overlays'
import { ProjectsPage } from './src/projects'
import { ReleasesPage } from './src/releases'
import {
  createKanbanTriggerRenderer,
  createPlanRenderer,
  createTicketMentionRenderer,
  createTicketRenderer,
} from './src/renderers'
import {
  BOARD_PAGE_ID,
  COLUMNS_CONFIGURATION_ID,
  PROJECTS_PAGE_ID,
  RELEASES_PAGE_ID,
  TICKET_PAGE_ID,
} from './src/shared'
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

  host.pages.register({
    id: RELEASES_PAGE_ID,
    title: 'Releases',
    configurationId: COLUMNS_CONFIGURATION_ID,
    render: (props) => <ReleasesPage {...props} host={host} />,
  })

  host.pages.register({
    id: PROJECTS_PAGE_ID,
    title: 'Projects',
    configurationId: COLUMNS_CONFIGURATION_ID,
    render: (props) => <ProjectsPage {...props} host={host} />,
  })

  // Palette rows that work while the page is closed: they open it and hand it the action to start.
  if (host.panels?.open) {
    const open = (pageId: string, action: string) => () => host.panels?.open({ pageId, context: { action } })
    const rows = (pageId: string, noun: string): PageCommand[] => [
      { id: `new-${noun}`, title: `New ${noun}`, run: open(pageId, 'new') },
      {
        id: `convert-${noun}`,
        title: 'Create from a label…',
        keywords: [noun, 'convert'],
        run: open(pageId, 'convert'),
      },
    ]
    host.commands?.register(RELEASES_PAGE_ID, rows(RELEASES_PAGE_ID, 'release'))
    host.commands?.register(PROJECTS_PAGE_ID, rows(PROJECTS_PAGE_ID, 'project'))
  }

  host.configForms.register(COLUMNS_CONFIGURATION_ID, KanbanConfigForm)
  host.functionTriggers.register(createTicketRenderer(host))
  host.functionTriggers.register(createPlanRenderer(host))
  host.triggerRenderers?.register(createKanbanTriggerRenderer(host))
  // `@kanban(id="…")` chat mentions preview as the ticket card.
  host.mentions?.registerRenderer(createTicketMentionRenderer(host))
}
