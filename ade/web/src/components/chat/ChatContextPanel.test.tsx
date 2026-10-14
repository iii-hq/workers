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
  return Promise.reject(new Error(`unexpected ${fn}`))
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
  }: { conversations?: Conversation[]; active?: boolean } = {},
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
          githubAvailable
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

  it('asks for a folder when the chat has none', async () => {
    const view = await render(chat('main'))
    expect(view.textContent).toContain('No project folder')
    expect(mocks.trigger).not.toHaveBeenCalled()
  })
})
