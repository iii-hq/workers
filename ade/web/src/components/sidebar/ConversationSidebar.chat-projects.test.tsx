// @vitest-environment jsdom

import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatProject } from '@/lib/backend/projects'
import type { Conversation } from '@/types/chat'
import { ConversationSidebar } from './ConversationSidebar'

function conversation(
  id: string,
  updatedAt: number,
  extra: Partial<Conversation> = {},
): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    hydrated: true,
    createdAt: 1,
    updatedAt,
    ...extra,
  }
}

const acme: ChatProject = {
  id: 'prj_acme',
  name: 'acme-web',
  color: 'teal',
  folders: [
    '/home/u/work/acme/web',
    '/home/u/work/acme/api',
    '/home/u/work/acme/infra',
  ],
  created_at_ms: 0,
  updated_at_ms: 0,
}
const mobile: ChatProject = {
  id: 'prj_mobile',
  name: 'mobile',
  color: 'rose',
  folders: ['/home/u/work/mobile'],
  created_at_ms: 0,
  updated_at_ms: 0,
}

const list: Conversation[] = [
  conversation('Webhook retries', 50, {
    projectId: 'prj_acme',
    workingDir: '/home/u/work/acme/web',
    description: 'Backoff with jitter for failed deliveries, then a PR',
    descriptionBy: 'agent',
  }),
  conversation('Flaky e2e on CI', 40, {
    projectId: 'prj_acme',
    workingDir: '/home/u/work/acme/web',
  }),
  conversation('Checkout copy pass', 30, { projectId: 'prj_acme' }),
  conversation('Fourth', 20, { projectId: 'prj_acme' }),
  conversation('Regex for log lines', 10, { workingDir: '/home/u/scratch' }),
]

function render(
  props: Partial<Parameters<typeof ConversationSidebar>[0]> = {},
) {
  return renderToStaticMarkup(
    <TooltipProvider>
      <ConversationSidebar
        conversations={list}
        activeId={null}
        onSelect={vi.fn()}
        onRename={vi.fn()}
        onRemove={vi.fn()}
        chatProjects={[acme, mobile]}
        onNewProject={vi.fn()}
        {...props}
      />
    </TooltipProvider>,
  )
}

afterEach(() => window.localStorage.clear())

describe('the list grouped by project (§1.3.1)', () => {
  it('groups by project by default, newest project first and No project last', () => {
    const html = render()
    const order = ['acme-web', 'mobile', 'No project'].map((name) =>
      html.indexOf(`>${name}<`),
    )
    expect(order.every((at) => at >= 0)).toBe(true)
    expect(order).toEqual([...order].sort((a, b) => a - b))
  })

  it('heads a project with its dot, name and folder count', () => {
    const html = render()
    expect(html).toContain('data-project-color="teal"')
    expect(html).toContain('3 folders')
    expect(html).toContain('1 folder<')
  })

  it('shows the 3 newest roots, then "1 more"', () => {
    const html = render()
    expect(html).toContain('aria-label="open Checkout copy pass"')
    expect(html).not.toContain('aria-label="open Fourth"')
    expect(html).toContain('1 more')
  })

  it('keeps an empty project with "No conversations yet"', () => {
    expect(render()).toContain('No conversations yet')
  })

  it('draws line 2: the description in Faint, else the folder; No project rows end with the folder', () => {
    const html = render()
    expect(html).toContain(
      'Backoff with jitter for failed deliveries, then a PR',
    )
    expect(html).toContain('data-kind="description"')
    expect(html).toContain('data-kind="folder"')
    expect(html).toContain('~/work/acme/web')
    expect(html).toContain('~/scratch')
  })

  it('puts New project next to New chat', () => {
    const html = render({ onCreate: vi.fn() })
    expect(html.indexOf('New project')).toBeGreaterThan(
      html.indexOf('New chat'),
    )
  })

  it('keeps the folder grouping and one-line rows on an engine without projects', () => {
    window.localStorage.setItem('iii-chat-grouping-chosen', '1')
    window.localStorage.setItem(
      'iii-chat-list-view',
      JSON.stringify({
        kinds: ['user'],
        grouping: 'project',
        sort: 'activity',
      }),
    )
    const html = render({ chatProjects: undefined })
    expect(html).not.toContain('New project')
    expect(html).not.toContain('No conversations yet')
    expect(html).toContain('No project')
    expect(html).not.toContain('data-kind="folder"')
  })
})
