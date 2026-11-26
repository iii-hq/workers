// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedChatProjectsForTests } from '@/hooks/use-chat-projects'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import type { ChatProject } from '@/lib/backend/projects'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import { setSessionListHasMoreForTests } from '@/lib/sessions/api'
import { usageFixture } from '@/stories/fixtures/claude-accounts-fixtures'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  /** The tab's loaded conversations: the sessions the counts come from. */
  conversations: [] as unknown[],
}))
// The stores are seeded; nothing here reads an engine.
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: vi.fn(async () => ({})) }),
}))
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => ({ conversations: mocks.conversations }),
}))

import { ClaudeAccountsDialog } from './ClaudeAccountsDialog'

const NOW = Date.now()

const PROJECTS: ChatProject[] = [
  {
    id: 'prj_web',
    name: 'acme-web',
    color: 'blue',
    folders: ['/w'],
    account: { mode: 'global' },
    created_at_ms: 0,
    updated_at_ms: 0,
  },
  {
    id: 'prj_billing',
    name: 'billing-api',
    color: 'green',
    folders: ['/b'],
    account: { mode: 'pinned', account_id: 'personal' },
    created_at_ms: 0,
    updated_at_ms: 0,
  },
]

const conversation = (
  id: string,
  extra: Record<string, unknown> = {},
): unknown => ({ id, status: 'done', ...extra })

let root: Root | null = null
let host: HTMLElement | null = null

beforeEach(() => {
  setChatCapabilitiesForTests({
    harnessRetry: true,
    tmuxAccounts: true,
    tmuxCredential: true,
    projects: false,
  })
  seedChatProjectsForTests(PROJECTS)
  seedClaudeAccountsForTests({ usage: usageFixture(NOW) })
  mocks.conversations = [
    conversation('w1', { projectId: 'prj_web', status: 'working' }),
    conversation('w2', { projectId: 'prj_web' }),
    conversation('w-child', { projectId: 'prj_web', parentId: 'w1' }),
    conversation('w-draft', { projectId: 'prj_web', draft: true }),
    conversation('b1', { projectId: 'prj_billing' }),
    conversation('n1'),
    conversation('n2', { projectId: 'prj_gone' }),
  ]
})

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  host = null
  setSessionListHasMoreForTests(false)
  seedClaudeAccountsForTests(null)
  seedChatProjectsForTests(null)
  setChatCapabilitiesForTests(null)
})

function render() {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ClaudeAccountsDialog
          open
          onOpenChange={vi.fn()}
          initialTab="projects"
        />
      </TooltipProvider>,
    )
  })
}

/** Each row's sessions cell, by row key (`none` is "No project"). */
function sessionCounts(): Record<string, string> {
  const rows = document.body.querySelectorAll('[data-project-account-row]')
  return Object.fromEntries(
    [...rows].map((row) => [
      row.getAttribute('data-project-account-row'),
      row.querySelectorAll('td')[3]?.textContent ?? '',
    ]),
  )
}

it('counts the sessions of each project and of no project', () => {
  render()
  expect(sessionCounts()).toEqual({
    prj_web: '2 · 1 working',
    prj_billing: '1',
    none: '2',
  })
})

it('shows "N+" on every row, "No project" too, when the list has more pages', () => {
  setSessionListHasMoreForTests(true)
  render()
  expect(sessionCounts()).toEqual({
    prj_web: '2+ · 1 working',
    prj_billing: '1+',
    none: '2+',
  })
  // The next reading came back whole.
  act(() => setSessionListHasMoreForTests(false))
  expect(sessionCounts().none).toBe('2')
})
