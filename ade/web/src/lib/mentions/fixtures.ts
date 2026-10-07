/**
 * A static mention runtime for Storybook and the mock surfaces: three
 * providers shaped like the real kanban, session-manager and console
 * (trace) ones, searched in memory. Install it with `setMentionRuntime`.
 */

import type { MentionRuntime } from './runtime'
import type { MentionItem, MentionProvider, MentionView } from './types'

export const FIXTURE_PROVIDERS: MentionProvider[] = [
  {
    v: 1,
    name: 'kanban',
    label: 'Tickets',
    description: 'A kanban ticket, by uuid or human key (KAN-12)',
    icon: 'ticket',
    color: 'blue',
    search: 'kanban::mention::search',
    details: { function_id: 'kanban::ticket::get', id_field: 'id' },
    getFunctionId: 'kanban::mention::get',
  },
  {
    v: 1,
    name: 'session',
    label: 'Sessions',
    description: 'A chat session, by session id',
    icon: 'session',
    color: 'neutral',
    search: 'session::mention::search',
    details: { function_id: 'session::get', id_field: 'session_id' },
    getFunctionId: 'session::mention::get',
  },
  {
    v: 1,
    name: 'trace',
    label: 'Traces',
    description: 'An engine trace, by trace id',
    icon: 'trace',
    color: 'teal',
    search: 'console::mentions::trace::search',
    details: { function_id: 'engine::traces::tree', id_field: 'trace_id' },
    getFunctionId: 'console::mentions::trace::get',
  },
]

export const FIXTURE_VIEWS: Record<string, MentionView[]> = {
  kanban: [
    {
      id: '6ac4f6df-ec84-83e9-b480-4b54b9931ce0',
      label: 'Fix login redirect loop',
      hint: 'KAN-12',
      description: 'In progress',
      color: 'rose',
      fields: [
        { label: 'Status', value: 'In progress' },
        { label: 'Priority', value: 'urgent', tone: 'danger' },
        { label: 'Assignee', value: 'backend-engineer' },
      ],
      open: { page: 'kanban-ticket', context: { id: 'KAN-12' } },
    },
    {
      id: '0f2a9c41-5b7e-4d0e-9c3a-2b1f8e6d4a10',
      label: 'Login page polish',
      hint: 'KAN-14',
      description: 'To do',
      color: 'blue',
      fields: [
        { label: 'Status', value: 'To do' },
        { label: 'Priority', value: 'medium' },
      ],
    },
  ],
  session: [
    {
      id: 'console-7d1e',
      label: 'investigate flaky login test',
      description: 'Done · 18 messages',
      color: 'green',
      fields: [
        { label: 'Status', value: 'Done', tone: 'success' },
        { label: 'Messages', value: '18' },
      ],
      open: { session: 'console-7d1e' },
    },
  ],
  trace: [
    {
      id: '4bf92f3577b34da6a3ce929d0e0e4736',
      label: 'harness::send',
      hint: '4bf92f35',
      description: 'error · 1.2 s · 14 spans · harness',
      color: 'rose',
      fields: [
        { label: 'Status', value: 'error', tone: 'danger' },
        { label: 'Duration', value: '1.2 s' },
        { label: 'Spans', value: '14' },
      ],
      open: {
        page: 'traces',
        context: { trace_id: '4bf92f3577b34da6a3ce929d0e0e4736' },
      },
    },
  ],
}

function asItem(view: MentionView): MentionItem {
  const { id, label, hint, description, icon, color } = view
  return { id, label, hint, description, icon, color }
}

export const fixtureMentionRuntime: MentionRuntime = {
  async listProviders() {
    return FIXTURE_PROVIDERS
  },
  async search(provider, { query, limit = 8 }) {
    const q = query.trim().toLowerCase()
    return (FIXTURE_VIEWS[provider.name] ?? [])
      .filter(
        (view) =>
          !q ||
          view.label.toLowerCase().includes(q) ||
          view.hint?.toLowerCase().includes(q),
      )
      .slice(0, limit)
      .map(asItem)
  },
  async get(provider, id) {
    return (
      (FIXTURE_VIEWS[provider.name] ?? []).find((view) => view.id === id) ??
      null
    )
  },
}
