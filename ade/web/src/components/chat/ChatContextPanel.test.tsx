// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { resetFoldedGroups } from '@/lib/context-groups'
import type { Conversation, Message } from '@/types/chat'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  trigger: vi.fn(),
  requestPanelOpen: vi.fn(),
  askAgent: vi.fn(),
  dismiss: vi.fn(),
  unregister: vi.fn(),
  autoFix: vi.fn(),
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
  if (fn === 'github::exec') {
    const args = payload.args as string[]
    const out = (value: unknown) =>
      Promise.resolve({
        value: { exit_code: 0, stdout: JSON.stringify(value), stderr: '' },
      })
    if (args[1] === 'list') {
      return out([
        {
          number: 9,
          title: 'Branch PR',
          url: 'https://github.com/o/r/pull/9',
          state: 'OPEN',
          headRefName: 'feat/x',
          additions: 447,
          deletions: 50,
          autoMergeRequest: null,
        },
      ])
    }
    if (args[1] === 'checks') return out([{ bucket: 'fail' }])
    if (args[0] === 'api') {
      return Promise.resolve({
        value: { exit_code: 0, stdout: 'me\n', stderr: '' },
      })
    }
    if (args[1] === 'view') {
      return out({
        state: 'OPEN',
        mergeable: 'MERGEABLE',
        reviews: [{ author: { login: 'bot' }, state: 'COMMENTED' }],
      })
    }
    return Promise.resolve({ value: { exit_code: 0, stdout: '', stderr: '' } })
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
    triggers = [],
  }: {
    conversations?: Conversation[]
    active?: boolean
    githubAvailable?: boolean
    kanbanAvailable?: boolean
    triggers?: SessionTriggerInfo[]
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
          triggers={triggers}
          onUnregisterTrigger={mocks.unregister}
          onOpenConversation={() => {}}
          shellAvailable
          githubAvailable={githubAvailable}
          kanbanAvailable={kanbanAvailable}
          active={active}
          onAskAgent={mocks.askAgent}
          onDismissPullRequest={mocks.dismiss}
          onAutoFixChange={mocks.autoFix}
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
  window.localStorage.clear()
  resetFoldedGroups()
  mocks.trigger.mockReset()
  mocks.trigger.mockImplementation(respond)
  mocks.requestPanelOpen.mockReset()
  mocks.askAgent.mockReset()
  mocks.dismiss.mockReset()
  mocks.unregister.mockReset()
  mocks.autoFix.mockReset()
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
    expect(text).toContain('1 failing')
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

  it('lists the pictures and files of the chat under Assets', async () => {
    const write = (
      id: string,
      functionId: string,
      path: string,
      output: unknown = { bytes_written: 5 },
    ): Message => ({
      id,
      role: 'function-trigger',
      functionId,
      input: { path, content: 'hello' },
      output,
      createdAt: 2,
    })
    const view = await render(
      chat('main', {
        workingDir: '/work/repo',
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
              {
                id: 'a2',
                name: 'spec.pdf',
                size: 2048,
                type: 'application/pdf',
                attachmentId: 'att-2',
              },
            ],
          },
          write('w1', 'shell::fs::write', '/tmp/out/report.html'),
          // Inside the folder: already among its changes.
          write('w2', 'shell::fs::write', '/work/repo/src/a.ts'),
          write('w3', 'shell::fs::write', '/tmp/out/failed.txt', {
            error: { kind: 'io' },
          }),
          write('w4', 'sandbox::fs::write', '/workspace/out.txt'),
        ],
      }),
    )
    const assets = view.querySelector('section[aria-label="Assets"]')
    expect(assets?.querySelector('button')?.textContent).toContain('3')
    const img = view.querySelector('img[src="data:image/png;base64,AAAA"]')
    expect(img?.closest('button')?.getAttribute('aria-label')).toBe(
      'View shot.png',
    )
    expect(assets?.textContent).toContain('2kb')
    expect(assets?.textContent).not.toContain('a.ts')
    expect(assets?.textContent).not.toContain('failed.txt')
    expect(assets?.textContent).not.toContain('out.txt')
    expect(
      view.querySelector('button[aria-label="Download spec.pdf"]'),
    ).not.toBeNull()
    act(() =>
      (
        view.querySelector(
          'button[aria-label="Open report.html in the IDE"]',
        ) as HTMLButtonElement
      ).click(),
    )
    expect(mocks.requestPanelOpen).toHaveBeenCalledWith({
      pageId: 'ide',
      context: { type: 'file', path: '/tmp/out/report.html' },
    })
  })

  it('folds a group to its brief and remembers it', async () => {
    const main = chat('main', {
      messages: [say('see https://example.com/a and https://docs.rs/b')],
    })
    let view = await render(main)
    const header = () =>
      view.querySelector(
        'section[aria-label="Links"] > button',
      ) as HTMLButtonElement
    expect(header().getAttribute('aria-expanded')).toBe('true')
    act(() => header().click())
    expect(header().getAttribute('aria-expanded')).toBe('false')
    expect(header().textContent).toContain('example.com · docs.rs')
    expect(view.querySelector('section[aria-label="Links"] a')).toBeNull()
    expect(byText('https://example.com/a')).toBeUndefined()

    act(() => root?.unmount())
    container?.remove()
    view = await render(main)
    expect(header().getAttribute('aria-expanded')).toBe('false')
    act(() => header().click())
    expect(header().getAttribute('aria-expanded')).toBe('true')
  })

  describe('pull request actions', () => {
    const open = async () => {
      const view = await render(chat('main', { workingDir: '/work/repo' }))
      const trigger = view.querySelector(
        'button[aria-label="Actions for o/r#9"]',
      ) as HTMLButtonElement
      await act(async () => {
        trigger.dispatchEvent(
          new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
        )
      })
      return view
    }
    const item = (name: string) =>
      [...document.querySelectorAll('[role^="menuitem"]')].find((el) =>
        el.textContent?.startsWith(name),
      ) as HTMLElement | undefined
    const ghCalls = () =>
      mocks.trigger.mock.calls
        .filter(([fn]) => fn === 'github::exec')
        .map(([, payload]) => (payload.args as string[]).join(' '))

    it('shows counts and failing checks, and offers the actions', async () => {
      const view = await open()
      const row = view.querySelector('section[aria-label="Pull requests"]')
      expect(row?.textContent).toContain('+447')
      expect(row?.textContent).toContain('−50')
      expect(row?.textContent).toContain('1 failing')
      expect(
        [...document.querySelectorAll('[role^="menuitem"]')].map(
          (el) => el.textContent,
        ),
      ).toEqual([
        'Open on GitHub',
        'Copy link',
        'View changes',
        'Auto-fix',
        'Merge when ready',
        'Convert to draft',
        'Dismiss',
      ])
    })

    it('turns merge when ready on with gh and reads the PR again', async () => {
      await open()
      await act(async () => item('Merge when ready')?.click())
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0))
      })
      expect(ghCalls()).toContain('pr merge 9 -R o/r --auto --squash')
      expect(ghCalls().filter((c) => c.startsWith('pr list'))).toHaveLength(2)
    })

    it('dismisses a PR', async () => {
      await open()
      await act(async () => item('Dismiss')?.click())
      expect(mocks.dismiss).toHaveBeenCalledWith('o/r#9')
    })

    it('starts auto-fix from where the PR stands, without the github worker', async () => {
      mocks.trigger.mockImplementation((fn: string, payload) =>
        fn === 'shell::exec' && payload.command === 'gh'
          ? respond('github::exec', payload).then(
              (r) => (r as { value: unknown }).value,
            )
          : respond(fn, payload),
      )
      const view = await render(chat('main', { workingDir: '/work/repo' }), {
        githubAvailable: false,
      })
      const trigger = view.querySelector(
        'button[aria-label="Actions for o/r#9"]',
      ) as HTMLButtonElement
      await act(async () => {
        trigger.dispatchEvent(
          new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
        )
      })
      expect(item('Auto-fix')?.getAttribute('data-disabled')).toBeNull()
      await act(async () => item('Auto-fix')?.click())
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0))
      })
      // Failures already there count as news; old reviews do not.
      expect(mocks.autoFix).toHaveBeenCalledWith('o/r#9', {
        url: 'https://github.com/o/r/pull/9',
        failing: [],
        reviews: 1,
        conflict: false,
        reports: 0,
      })
    })

    it('shows auto-fix on from the session and turns it off', async () => {
      const view = await render(
        chat('main', {
          workingDir: '/work/repo',
          sessionMetadata: {
            context_autofix: {
              'o/r#9': {
                url: 'https://github.com/o/r/pull/9',
                failing: [],
                reviews: 0,
                conflict: false,
                reports: 0,
              },
            },
          },
        }),
      )
      const trigger = view.querySelector(
        'button[aria-label="Actions for o/r#9"]',
      ) as HTMLButtonElement
      await act(async () => {
        trigger.dispatchEvent(
          new PointerEvent('pointerdown', { bubbles: true, button: 0 }),
        )
      })
      expect(item('Auto-fix')?.getAttribute('aria-checked')).toBe('true')
      await act(async () => item('Auto-fix')?.click())
      expect(mocks.autoFix).toHaveBeenCalledWith('o/r#9', null)
    })

    it('offers to create a PR for a branch with work and none of its own', async () => {
      mocks.trigger.mockImplementation((fn: string, payload) =>
        fn === 'github::exec'
          ? Promise.resolve({
              value: { exit_code: 0, stdout: '[]', stderr: '' },
            })
          : respond(fn, payload),
      )
      const view = await render(chat('main', { workingDir: '/work/repo' }))
      const create = [...view.querySelectorAll('button')].find((el) =>
        el.textContent?.startsWith('Create pull request'),
      )
      expect(create).toBeDefined()
      act(() => create?.click())
      expect(mocks.askAgent).toHaveBeenCalledWith(
        expect.stringContaining('open a pull request'),
        false,
      )
    })

    it('offers no PR creation when the branch could not be read', async () => {
      mocks.trigger.mockImplementation((fn: string, payload) =>
        fn === 'github::exec'
          ? Promise.resolve({
              value: { exit_code: 4, stdout: '', stderr: 'gh auth login' },
            })
          : respond(fn, payload),
      )
      const view = await render(chat('main', { workingDir: '/work/repo' }))
      expect(view.textContent).not.toContain('Create pull request')
    })

    it('offers no PR creation while the branch has an open one', async () => {
      const view = await render(chat('main', { workingDir: '/work/repo' }))
      expect(view.textContent).not.toContain('Create pull request')
    })
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
        'Open KAN-141 Filters by label in a new panel (In review, high priority)',
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
