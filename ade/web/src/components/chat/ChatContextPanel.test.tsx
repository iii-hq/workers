// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { Conversation, Message } from '@/types/chat'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  trigger: vi.fn(),
  requestPanelOpen: vi.fn(),
}))

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ trigger: mocks.trigger }),
}))
vi.mock('@/lib/panel-context', () => ({
  requestPanelOpen: mocks.requestPanelOpen,
}))

import { ChatContextPanel } from './ChatContextPanel'

const GIT: Record<string, string> = {
  status: '## feat/x...origin/feat/x [ahead 2]\n M a.ts\n?? b.ts\n',
  diff: '5\t1\ta.ts\n',
  remote: 'git@github.com:o/r.git\n',
}

function respond(fn: string, payload: Record<string, unknown> = {}) {
  if (fn === 'shell::exec') {
    const args = payload.args as string[]
    const verb = args.find((arg) => arg in GIT) ?? ''
    return Promise.resolve({ exit_code: 0, stdout: GIT[verb] ?? '' })
  }
  if (fn === 'github::pr::list') {
    return Promise.resolve({
      value: [
        {
          number: 9,
          title: 'Branch PR',
          url: 'https://github.com/o/r/pull/9',
          state: 'OPEN',
          headRefName: 'feat/x',
        },
      ],
    })
  }
  if (fn === 'github::pr::checks') {
    return Promise.resolve({ value: [{ bucket: 'fail' }] })
  }
  if (fn === 'kanban::config::info') {
    return Promise.resolve({
      columns: [
        { id: 'in_review', label: 'In review' },
        { id: 'done', label: 'Done' },
      ],
    })
  }
  if (fn === 'kanban::ticket::list') {
    return Promise.resolve({
      tickets: [
        ticketRow('u-141', 'KAN-141', 'Filters by label', 'in_review', 'high'),
        ticketRow('u-117', 'KAN-117', 'Empty board', 'done', 'low'),
      ],
    })
  }
  return Promise.reject(new Error(`unexpected ${fn}`))
}

function ticketRow(
  id: string,
  key: string,
  title: string,
  status: string,
  priority: string,
) {
  return { id, key, title, status, priority, deleted_at: null }
}

function chat(id: string, extra: Partial<Conversation> = {}): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    createdAt: 1,
    updatedAt: Date.now(),
    ...extra,
  }
}

const say = (content: string): Message => ({
  id: `m-${content.length}`,
  role: 'assistant',
  content,
  createdAt: 1,
})

let root: Root | null = null
let container: HTMLElement | null = null

async function render(
  main: Conversation,
  {
    conversations = [main],
    active = true,
    githubAvailable = true,
    kanbanAvailable = false,
  }: {
    conversations?: Conversation[]
    active?: boolean
    githubAvailable?: boolean
    kanbanAvailable?: boolean
  } = {},
) {
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
  await act(async () => {
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ChatContextPanel
          conversation={main}
          conversations={conversations}
          connectionState="connected"
          triggers={[]}
          onUnregisterTrigger={() => {}}
          onOpenConversation={() => {}}
          shellAvailable
          githubAvailable={githubAvailable}
          kanbanAvailable={kanbanAvailable}
          active={active}
        />
      </TooltipProvider>,
    )
  })
  // The git read, then the PR read it unlocks, then their state updates.
  for (let i = 0; i < 4; i += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
  return container
}

const byText = (text: string) =>
  [...(container?.querySelectorAll('*') ?? [])].find(
    (el) => el.childElementCount === 0 && el.textContent?.trim() === text,
  ) as HTMLElement | undefined

beforeEach(() => {
  mocks.trigger.mockReset()
  mocks.trigger.mockImplementation(respond)
  mocks.requestPanelOpen.mockReset()
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
})

describe('ChatContextPanel', () => {
  it('shows the folder, branch, grouped changes and the branch PR', async () => {
    const view = await render(chat('main', { workingDir: '/work/repo' }))
    const text = view.textContent ?? ''
    expect(text).toContain('repo')
    expect(text).toContain('feat/x')
    expect(text).toContain('2 ahead')
    expect(text).toContain('2 changed files')
    expect(text).toContain('+5')
    expect(text).toContain('−1')
    expect(text).toContain('Branch PR')
    expect(text).toContain('#9 · open')
    expect(text).toContain('checks failing')
  })

  it('opens the IDE source control from the changes row', async () => {
    await render(chat('main', { workingDir: '/work/repo' }))
    act(() => byText('2 changed files')?.click())
    expect(mocks.requestPanelOpen).toHaveBeenCalledWith({
      pageId: 'ide',
      context: { type: 'source-control', cwd: '/work/repo' },
    })
  })

  it('keeps row status out of the slot the recipe hides on hover', async () => {
    await render(chat('main', { workingDir: '/work/repo' }))
    for (const text of ['+5', '2 ahead']) {
      expect(byText(text)?.closest('.iii-ui-tree-item__trailing')).toBeNull()
    }
  })

  it('never nests a button inside a button-role row', async () => {
    const view = await render(
      chat('main', {
        workingDir: '/work/repo',
        messages: [say('see https://docs.example.com/guide')],
      }),
    )
    expect(view.querySelectorAll('[role="button"] button')).toHaveLength(0)
    expect(view.querySelectorAll('button button')).toHaveLength(0)
  })

  it('caps sub-agents at five and expands the rest in place', async () => {
    const main = chat('main')
    const children = Array.from({ length: 8 }, (_, i) =>
      chat(`id-${i + 1}`, {
        title: `helper-${i + 1}`,
        parentId: 'main',
        status: 'done',
      }),
    )
    const view = await render(main, { conversations: [main, ...children] })
    expect(view.textContent).toContain('Show 3 more')
    expect(view.textContent).not.toContain('helper-8')
    act(() => byText('Show 3 more')?.click())
    expect(view.textContent).toContain('helper-8')
    expect(view.textContent).toContain('Show less')
  })

  it('lists sub-agents of every depth, running ones first', async () => {
    const main = chat('main')
    const done = chat('d', {
      title: 'done-child',
      parentId: 'main',
      status: 'done',
    })
    const running = chat('r', {
      title: 'running-grandchild',
      parentId: 'd',
      status: 'working',
    })
    const view = await render(main, { conversations: [main, done, running] })
    const text = view.textContent ?? ''
    expect(text).toContain('running-grandchild')
    expect(text.indexOf('running-grandchild')).toBeLessThan(
      text.indexOf('done-child'),
    )
  })

  it('survives a link with a malformed escape', async () => {
    const view = await render(
      chat('main', {
        messages: [say('call https://api.example.com/users/%s now')],
      }),
    )
    expect(view.textContent).toContain('users/%s')
  })

  it('reads nothing while the panel is out of sight', async () => {
    await render(chat('main', { workingDir: '/work/repo' }), { active: false })
    expect(mocks.trigger).not.toHaveBeenCalled()
  })

  it('reads the branch PR with gh when the github worker is absent', async () => {
    mocks.trigger.mockImplementation(
      (fn: string, payload: Record<string, unknown> = {}) => {
        if (fn === 'shell::exec' && payload.command === 'gh') {
          return Promise.resolve({
            exit_code: 0,
            stdout: JSON.stringify([
              {
                number: 12,
                title: 'Via gh',
                url: 'https://github.com/o/r/pull/12',
                state: 'MERGED',
                headRefName: 'feat/x',
              },
            ]),
          })
        }
        return respond(fn, payload)
      },
    )
    const view = await render(chat('main', { workingDir: '/work/repo' }), {
      githubAvailable: false,
    })
    expect(view.textContent).toContain('Via gh')
    expect(
      mocks.trigger.mock.calls.some(([fn]) => fn.startsWith('github::')),
    ).toBe(false)
  })

  it('shows the pictures of the chat as a grid that opens them', async () => {
    const view = await render(
      chat('main', {
        messages: [
          {
            id: 'u1',
            role: 'user',
            content: 'see',
            createdAt: 1,
            attachments: [
              {
                id: 'a1',
                name: 'shot.png',
                size: 3,
                type: 'image/png',
                dataUrl: 'data:image/png;base64,AAAA',
              },
            ],
          },
        ],
      }),
    )
    expect(view.textContent).toContain('Images')
    const img = view.querySelector('img[src="data:image/png;base64,AAAA"]')
    expect(img).not.toBeNull()
    expect(img?.closest('button')?.getAttribute('aria-label')).toBe(
      'View shot.png',
    )
  })

  describe('tickets', () => {
    const worked: Message[] = [
      {
        id: 'f1',
        role: 'function-trigger',
        functionId: 'kanban::ticket::move',
        input: { id: 'KAN-141', status: 'in_review' },
        output: { key: 'KAN-141' },
        createdAt: Date.now() - 12 * 60_000,
      },
      say('Done with KAN-141; KAN-117 stays as is, and UTF-8 is fine.'),
    ]

    it('lists worked tickets first and opens one in a Kanban pane', async () => {
      const view = await render(chat('main', { messages: worked }), {
        kanbanAvailable: true,
      })
      const section = view.querySelector('section[aria-label="Tickets"]')
      expect(section?.textContent).toContain('Filters by label')
      expect(section?.textContent).toContain('moved here 12m ago')
      expect(section?.textContent).toContain('mentioned')
      expect(section?.textContent).not.toContain('UTF-8')
      const rows = [
        ...(section?.querySelectorAll('button[aria-label^="Open"]') ?? []),
      ]
      expect(rows.map((row) => row.getAttribute('aria-label'))).toEqual([
        'Open KAN-141 Filters by label in a new panel (In review)',
        'Open KAN-117 Empty board in a new panel (Done)',
      ])
      expect(
        section?.querySelector('[aria-label="High priority"]'),
      ).not.toBeNull()
      act(() => (rows[0] as HTMLButtonElement).click())
      expect(mocks.requestPanelOpen).toHaveBeenCalledWith({
        pageId: 'kanban-ticket',
        context: { id: 'KAN-141' },
      })
    })

    it('shows nothing without the kanban worker or a ticket the board knows', async () => {
      let view = await render(chat('main', { messages: worked }))
      expect(view.querySelector('section[aria-label="Tickets"]')).toBeNull()
      act(() => root?.unmount())
      container?.remove()
      view = await render(
        chat('main', { messages: [say('UTF-8 and ISO-8601')] }),
        {
          kanbanAvailable: true,
        },
      )
      expect(view.querySelector('section[aria-label="Tickets"]')).toBeNull()
    })

    it('holds a skeleton while the board is read, only for a kanban call', async () => {
      mocks.trigger.mockImplementation((fn: string, payload) =>
        fn.startsWith('kanban::')
          ? new Promise(() => {})
          : respond(fn, payload),
      )
      let view = await render(chat('main', { messages: worked }), {
        kanbanAvailable: true,
      })
      expect(
        view.querySelector('section[aria-label="Tickets"] [aria-hidden]'),
      ).not.toBeNull()
      act(() => root?.unmount())
      container?.remove()
      view = await render(chat('main', { messages: [say('see KAN-117')] }), {
        kanbanAvailable: true,
      })
      expect(view.querySelector('section[aria-label="Tickets"]')).toBeNull()
    })
  })

  it('asks for a folder when the chat has none', async () => {
    const view = await render(chat('main'))
    expect(view.textContent).toContain('No project folder')
    expect(mocks.trigger).not.toHaveBeenCalled()
  })
})
