/**
 * The kanban worker's native app for the iii app (ide/projects/
 * kanban-native-ui/design.md): the board as lanes, a ticket with its
 * conversation, new tickets, filters, and the releases and projects to read.
 * Published by src/native_ui.rs as kanban/native.js; it needs catalogue 2
 * (Markdown, Lanes, named hues, drag and drop, ListDetail.listFill).
 */

import { defineApp } from '@iii-dev/native-ui'
import { FiltersPage } from './pages/filters'
import { NewPage } from './pages/new'
import { PickPage } from './pages/pick'
import { ProjectsPage, ReleasesPage } from './pages/plan'
import { RootPage } from './pages/root'
import { TextPage } from './pages/text'
import { TicketPage } from './pages/ticket'

defineApp({
  catalog: 2,
  pages: {
    root: RootPage,
    ticket: TicketPage,
    new: { component: NewPage, modal: true },
    text: TextPage,
    pick: PickPage,
    filters: FiltersPage,
    releases: ReleasesPage,
    projects: ProjectsPage,
  },
})
