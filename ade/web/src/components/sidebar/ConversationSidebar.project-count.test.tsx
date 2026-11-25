// @vitest-environment jsdom

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatProject } from '@/lib/backend/projects'
import type { Conversation } from '@/types/chat'

// The menu itself is ProjectMenu's own suite; here it shows the text its
// Delete confirmation would read for the count the list hands it.
vi.mock('@/components/projects/ProjectMenu', async (original) => {
  const actual =
    await original<typeof import('@/components/projects/ProjectMenu')>()
  return {
    ...actual,
    ProjectMenu: ({
      project,
      conversationCount,
    }: {
      project: ChatProject
      conversationCount: number
    }) => (
      <span data-delete-text={project.name}>
        {actual.deleteProjectText(conversationCount)}
      </span>
    ),
  }
})

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
  color: 'blue',
  folders: ['/home/u/work/acme/web', '/home/u/work/acme/api'],
  created_at_ms: 0,
  updated_at_ms: 0,
}

// acme-web after bring_existing: 4 conversations, one with a subagent.
const list: Conversation[] = [
  conversation('Upgrade React to 19', 50, { projectId: 'prj_acme' }),
  conversation('Terraform the staging cluster', 40, { projectId: 'prj_acme' }),
  conversation('Rate limit the public API', 30, { projectId: 'prj_acme' }),
  conversation('Fix email validation in the login form', 20, {
    projectId: 'prj_acme',
  }),
  conversation('Find where emails are validated', 25, {
    parentId: 'Fix email validation in the login form',
    hierarchyResolved: true,
  }),
  conversation('Regex for plus addresses', 10),
]

describe('a project is counted as the list counts it (P11)', () => {
  it('Delete confirmation: roots only, a subagent is not counted', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ConversationSidebar
          conversations={list}
          activeId={null}
          onSelect={vi.fn()}
          onRename={vi.fn()}
          onRemove={vi.fn()}
          chatProjects={[acme]}
          onNewProject={vi.fn()}
        />
      </TooltipProvider>,
    )
    expect(html).toContain('Its 4 conversations stay, under No project.')
    expect(html).not.toContain('Its 5 conversations')
  })
})
