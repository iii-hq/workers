// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider, host } from './test-ui'
vi.mock('@iii-dev/console-ui', async () => (await import('./test-ui')).mockUi)
import type { DecisionChatHandle, DecisionChatRequest, Host } from '@iii-dev/console-ui'
type IiiClient = Host['iii']
import { TowerScreen, type TowerScreenProps } from '.'
import { towerFixture } from './fixtures'
import { NeedsYou } from './NeedsYou'
import { TowerHost } from './host'
import type { AskEntry, QuestionNeed } from '../../src/types'

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
let root: Root
let container: HTMLDivElement
beforeEach(() => {
  localStorage.clear()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
})
async function render(props: Partial<TowerScreenProps> = {}) {
  await act(() =>
    root.render(
      <TooltipProvider>
        <TowerScreen
          host={host}
          data={towerFixture}
          scope="all"
          onScopeChange={() => {}}
          initialView="needs-you"
          {...props}
        />
      </TooltipProvider>,
    ),
  )
}
function button(label: string) {
  const found = [...container.querySelectorAll('button')].find((node) =>
    label === 'Ask about this' ? node.textContent?.startsWith(label) : node.textContent?.trim() === label,
  )
  expect(found, label).toBeDefined()
  return found as HTMLButtonElement
}
async function click(label: string) {
  await act(() => button(label).click())
}
const question = towerFixture.items[0] as QuestionNeed
const client = (call: ReturnType<typeof vi.fn>) => ({
  call: call as IiiClient['call'],
})

describe('Tower Needs you interactions', () => {
  it('replaces choices with the exact sent summary and sends only once on two clicks', async () => {
    let deliver: (entry: AskEntry) => void = () => {}
    const call = vi.fn(
      () =>
        new Promise<AskEntry>((resolve) => {
          deliver = resolve
        }),
    )
    const refresh = vi.fn()
    await render({ iii: client(call), data: { ...towerFixture, refresh } })
    const answer = button('Answer')
    expect(answer.disabled).toBe(false)
    expect(container.querySelectorAll('input:checked')).toHaveLength(1)
    await act(() => {
      answer.click()
      answer.click()
    })
    expect(call).toHaveBeenCalledTimes(1)
    expect(call).toHaveBeenCalledWith('harness::ask::answer', {
      session_id: question.sessionId,
      question_id: question.questionId,
      answers: [
        {
          header: 'Builds',
          choice: question.entry.questions[0].options[0].label,
        },
      ],
      by: { channel: 'tower', user: 'user' },
    })
    expect(container.textContent).toContain('Sending your answer')
    expect(container.querySelector('.tower-options')).toBeNull()
    await act(() =>
      deliver({
        ...question.entry,
        state: 'answered',
        answer_text: 'Builds: Isolate builds',
        by: { channel: 'tower', user: 'user' },
        outcome: 'answered',
      }),
    )
    expect(container.textContent).toContain('Delivered')
    expect(refresh).toHaveBeenCalledOnce()
  })
  it('retries the original session answer after a failed send despite reused question ids', async () => {
    const call = vi
      .fn()
      .mockRejectedValueOnce(new Error('transport timeout'))
      .mockResolvedValueOnce({
        questions: [
          { harness: { ...question.entry, session_id: 'another-session', state: 'answered' } },
          { harness: question.entry },
        ],
      })
      .mockResolvedValueOnce({ ...question.entry, state: 'answered' })
    await render({ iii: client(call) })
    await click('Answer')
    expect(container.textContent).toContain('Your answer did not reach the harness')
    expect(container.querySelectorAll('input:checked')).toHaveLength(1)
    await click('Retry')
    expect(call.mock.calls[1]).toEqual(['tower::overview', { scope: 'all' }])
    expect(call.mock.calls[2]).toEqual(call.mock.calls[0])
    expect(container.textContent).toContain('Delivered')
  })
  it('keeps an expiry delivery race open and retries the answer shortly', async () => {
    vi.useFakeTimers()
    const call = vi
      .fn()
      .mockRejectedValueOnce(new Error('ask_busy: expiry notice in delivery, retry'))
      .mockResolvedValueOnce({ ...question.entry, state: 'answered' })
    await render({ iii: client(call) })
    await click('Answer')
    expect(container.textContent).toContain('Sending your answer')
    expect(container.textContent).not.toContain('Already answered')
    expect(call).toHaveBeenCalledTimes(1)
    await act(async () => vi.advanceTimersByTimeAsync(500))
    expect(call).toHaveBeenCalledTimes(2)
    expect(call.mock.calls[1]).toEqual(call.mock.calls[0])
    expect(container.textContent).toContain('Delivered')
    vi.useRealTimers()
  })
  it('stops retrying an expiry delivery race after the question closes', async () => {
    vi.useFakeTimers()
    const call = vi.fn().mockRejectedValue(new Error('ask_busy: expiry notice in delivery, retry'))
    await render({ iii: client(call) })
    await click('Answer')
    await render({
      iii: client(call),
      data: {
        ...towerFixture,
        items: towerFixture.items.filter((item) => item.id !== question.id),
      },
    })
    await act(async () => vi.advanceTimersByTimeAsync(500))
    expect(call).toHaveBeenCalledTimes(1)
    vi.useRealTimers()
  })
  it('reconciles a reserved answer on Retry without sending a second answer', async () => {
    const call = vi
      .fn()
      .mockRejectedValueOnce(new Error('delivery timeout'))
      .mockResolvedValueOnce({
        questions: [
          {
            harness: {
              ...question.entry,
              state: 'answering',
              answer_text: 'Builds: Sandbox',
              by: { channel: 'tower', user: 'user' },
            },
          },
        ],
      })
    await render({ iii: client(call) })
    await click('Answer')
    await click('Retry')
    expect(call.mock.calls.filter(([id]) => id === 'harness::ask::answer')).toHaveLength(1)
    expect(container.textContent).toContain('Sending your answer')
    expect(container.querySelector('.tower-options')).toBeNull()
  })
  it('removes a held detail when another tab resolves it', async () => {
    await render({ iii: client(vi.fn()), initialSelectedId: 'hold' })
    expect(button('Allow once')).toBeDefined()
    await render({
      data: {
        ...towerFixture,
        items: towerFixture.items.filter((item) => item.kind !== 'held'),
      },
    })
    expect(container.textContent).toContain('Choose an item')
    expect([...container.querySelectorAll('button')].some((node) => node.textContent === 'Allow once')).toBe(false)
  })
  it('lists releases beyond the Overview selection and filters their open items', async () => {
    const release = {
      ...towerFixture.releases[0],
      id: 'older',
      name: 'Older release',
    }
    const item = {
      ...towerFixture.items[0],
      card: { ...towerFixture.tickets[0], release_id: release.id },
    }
    await render({
      data: {
        ...towerFixture,
        releaseCatalog: [...towerFixture.releaseCatalog, release],
        items: [item],
      },
    })
    const rail = container.querySelector('.tower-needs-rail')
    const row = [...(rail?.querySelectorAll('button') ?? [])].find((node) =>
      node.textContent?.includes('Older release'),
    )
    expect(row).toBeDefined()
    await act(() => row?.click())
    expect(container.querySelectorAll('.tower-need-row')).toHaveLength(1)
  })
  it('closes the selected question when the shared data reports another channel or withdrawal', async () => {
    await render({ iii: client(vi.fn()) })
    const settled = {
      ...question.entry,
      state: 'answered' as const,
      by: { channel: 'kanban', user: 'person' },
      answer_text: 'Use the sandbox',
      outcome: 'answered' as const,
    }
    const data = {
      ...towerFixture,
      items: towerFixture.items.filter((item) => item.id !== question.id),
      questions: towerFixture.questions.map((row) =>
        row.questionId === question.questionId ? { ...row, state: settled.state, harness: settled } : row,
      ),
    }
    await render({ data })
    expect(container.textContent).toContain('Already answered by kanban')
    expect(container.querySelector('.tower-options')).toBeNull()
    expect([...container.querySelectorAll('button')].some((node) => node.textContent === 'Answer')).toBe(false)
    await render({
      data: {
        ...data,
        questions: data.questions.map((row) =>
          row.questionId === question.questionId
            ? {
                ...row,
                harness: {
                  ...settled,
                  state: 'withdrawn',
                  outcome: 'withdrawn',
                },
              }
            : row,
        ),
      },
    })
    expect(container.textContent).toContain('Withdrawn by the asker')
  })
  it('has no default for an unmarked question and only offers dismissal for permitted classes', async () => {
    await render({ iii: client(vi.fn()), initialSelectedId: 'csv' })
    expect(button('Answer').disabled).toBe(true)
    expect(container.querySelectorAll('input:checked')).toHaveLength(0)
    await act(() => container.querySelector<HTMLInputElement>('input[type="radio"]')?.click())
    expect(button('Answer').disabled).toBe(false)
    expect(button('Let the agent decide')).toBeDefined()
    await render({
      data: {
        ...towerFixture,
        questions: towerFixture.questions.map((row) => ({
          ...row,
          harness: undefined,
        })),
      },
    })
    expect(container.textContent).toContain('Expiry policy is unavailable until the harness returns.')
  })
  it('sends dismissed for scope questions and never offers it for security', async () => {
    const call = vi.fn().mockResolvedValue({
      ...(towerFixture.items.find((item) => item.id === 'csv') as QuestionNeed).entry,
      state: 'dismissed',
    })
    await render({ iii: client(call), initialSelectedId: 'csv' })
    await click('Let the agent decide')
    expect(call).toHaveBeenCalledWith('harness::ask::answer', {
      session_id: 'import-tool',
      question_id: 'csv',
      dismissed: true,
      by: { channel: 'tower', user: 'user' },
    })
  })
  it('shows held arguments without options or preselection, and grants before resolving always', async () => {
    const call = vi.fn().mockResolvedValue({ resolved: true })
    const held = towerFixture.items.find((item) => item.kind === 'held')
    if (!held) throw new Error('Held fixture missing')
    await render({
      iii: client(call),
      initialSelectedId: held.id,
      data: {
        ...towerFixture,
        items: towerFixture.items.map((item) =>
          item.id === held.id ? { ...item, namespace: 'production', implicitNamespace: 'default' } : item,
        ),
      },
    })
    expect(container.querySelector('input[type="radio"],input[type="checkbox"]')).toBeNull()
    expect(container.textContent).not.toContain('Recommended')
    expect(container.textContent).not.toContain('Let the agent decide')
    expect(container.textContent).toContain('git push')
    expect(container.textContent).toContain('Namespace: production')
    expect(container.querySelector('.tower-decision button[data-variant="default"]')).toBeNull()
    await click('Always allow in this session')
    expect(call.mock.calls).toEqual([
      [
        'approval::approve-always',
        {
          session_id: held.sessionId,
          function_id: 'shell::exec',
          namespace: 'production',
        },
      ],
      [
        'approval::resolve',
        {
          session_id: held.sessionId,
          function_call_id: 'push',
          decision: 'allow',
          namespace: 'production',
        },
      ],
    ])
  })
  it.each(['Deny', 'Allow once'])('uses an explicit %s choice for a held call', async (choice) => {
    const call = vi.fn().mockResolvedValue({ resolved: true })
    await render({ iii: client(call), initialSelectedId: 'hold' })
    await click(choice)
    expect(call).toHaveBeenCalledWith('approval::resolve', {
      session_id: 'dispatch-r1',
      function_call_id: 'push',
      decision: choice === 'Deny' ? 'deny' : 'allow',
    })
  })
  it('resumes the stalled root with continue', async () => {
    const call = vi.fn().mockResolvedValue({})
    await render({ iii: client(call), initialSelectedId: 'stall' })
    await click('Resume')
    expect(call).toHaveBeenCalledWith('harness::send', {
      session_id: 'dispatch-flaky',
      message: 'continue',
    })
  })
  it('leaves a stalled item hidden until its run changes', async () => {
    await render({ initialSelectedId: 'stall' })
    await click('Leave it')
    expect(container.querySelectorAll('.tower-need-row')).toHaveLength(towerFixture.items.length - 1)
    await render({
      data: {
        ...towerFixture,
        runs: towerFixture.runs.map((run) =>
          run.session.session_id === 'dispatch-flaky'
            ? { ...run, status: { ...run.status, turn_id: 'new-turn' } }
            : run,
        ),
      },
    })
    expect(container.querySelectorAll('.tower-need-row')).toHaveLength(towerFixture.items.length)
  })
})
describe('Tower question logs', () => {
  it('posts a person comment to reopen an assumption and keeps it locally', async () => {
    const call = vi.fn().mockResolvedValue({})
    await render({ iii: client(call), initialView: 'assumptions' })
    await click('Reopen as a question')
    expect(call).toHaveBeenCalledWith('kanban::comment::create', {
      ticket_id: 'KAN-597',
      author: 'user',
      body: 'Reopen as a question:\nCache artifacts between builds?\n\nPlease revisit the recorded assumption: Keep the cache isolated per project.',
    })
    expect(container.textContent).toContain('Comment posted')
    await click('Keep')
    expect(container.textContent).toContain('No assumptions to review')
    await click('Overview')
    await click('Assumptions')
    expect(container.textContent).toContain('No assumptions to review')
  })
  it('lists closed questions, not held calls or recorded assumptions', async () => {
    await render({ initialView: 'decided' })
    expect(container.textContent).toContain('May builds reach the public network?')
    expect(container.textContent).toContain('Allow only the package mirrors.')
    expect(container.textContent).toContain('not delivered')
    expect(button('Open card')).toBeDefined()
    expect(container.textContent).not.toContain('shell::exec')
    expect(container.textContent).not.toContain('Cache artifacts between builds?')
  })
})

describe('Tower decision capability', () => {
  it('detaches a stale rebind that resolves after Ask about this opens the copy', async () => {
    let resolve: (handle: DecisionChatHandle | null) => void = () => {}
    const stale = new Promise<DecisionChatHandle | null>((done) => {
      resolve = done
    })
    const opened: DecisionChatHandle = {
      update: vi.fn(),
      detach: vi.fn(),
      close: vi.fn(),
    }
    const delayed: DecisionChatHandle = {
      update: vi.fn(),
      detach: vi.fn(),
      close: vi.fn(),
    }
    const chat = {
      openDecisionChat: vi.fn(async () => opened),
      rebindDecisionChat: vi.fn(() => stale),
    }
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            host={host}
            data={towerFixture}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
            iii={{ call: vi.fn() as IiiClient['call'] }}
            chat={chat}
          />
        </TooltipProvider>,
      ),
    )
    expect(chat.rebindDecisionChat).toHaveBeenCalled()
    await click('Ask about this')
    expect(chat.openDecisionChat).toHaveBeenCalled()
    await act(async () => {
      resolve(delayed)
      await stale
    })
    expect(delayed.detach).toHaveBeenCalled()
    expect(opened.detach).not.toHaveBeenCalled()
  })

  it('rebinds every displayed decision, closes removed items, and detaches on unmount', async () => {
    const bindings = [
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
    ]
    const handles = [...bindings]
    const chat = {
      openDecisionChat: vi.fn(),
      rebindDecisionChat: vi.fn(async () => handles.shift() ?? null),
    }
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            host={host}
            data={towerFixture}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
            chat={chat}
          />
        </TooltipProvider>,
      ),
    )
    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })
    expect(chat.rebindDecisionChat).toHaveBeenCalledTimes(
      towerFixture.items.filter((item) => item.kind !== 'stalled').length,
    )
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            host={host}
            data={{ ...towerFixture, items: towerFixture.items.slice(0, 1) }}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
            chat={chat}
          />
        </TooltipProvider>,
      ),
    )
    expect(bindings[1].update).toHaveBeenCalledWith({
      editable: false,
      closed: true,
    })
    await act(() => root.unmount())
    expect(bindings[0].detach).toHaveBeenCalled()
    expect(bindings[1].detach).toHaveBeenCalled()
  })

  it.each(['option', 'text'])('fills a %s through the host request, then only Answer sends it', async (kind) => {
    let opened: DecisionChatRequest | undefined
    const chat = {
      openDecisionChat: vi.fn(async (request: DecisionChatRequest) => {
        opened = request
        return { update: vi.fn(), detach: vi.fn(), close: vi.fn() }
      }),
      rebindDecisionChat: vi.fn(async () => null),
    }
    const call = vi.fn(async () => ({ ...question.entry, state: 'answered' }))
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            host={host}
            data={towerFixture}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
            iii={{ call: call as IiiClient['call'] }}
            chat={chat}
          />
        </TooltipProvider>,
      ),
    )
    await click('Ask about this')
    const proposal =
      kind === 'option'
        ? { option: question.entry.questions[0].options[1].label }
        : { text: 'Use a dedicated build machine.' }
    await act(() => opened?.onUseAnswer(0, proposal))
    await click('Answer')
    expect(call).toHaveBeenCalledWith(
      'harness::ask::answer',
      expect.objectContaining({
        answers: [
          {
            header: 'Builds',
            ...(kind === 'option'
              ? { choice: question.entry.questions[0].options[1].label }
              : { other: 'Use a dedicated build machine.' }),
          },
        ],
      }),
    )
  })

  it('hides Ask about this without the capability', async () => {
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen host={host} data={towerFixture} scope="all" onScopeChange={() => {}} initialView="needs-you" />
        </TooltipProvider>,
      ),
    )
    expect(container.textContent).not.toContain('Ask about this')
  })
})

describe('decision bindings from Overview', () => {
  it('rebinds open copies on Overview and opens the correct draft when their answer is used', async () => {
    const requests: DecisionChatRequest[] = []
    const chat = {
      rebindDecisionChat: vi.fn(async (request: DecisionChatRequest) => {
        requests.push(request)
        return { update: vi.fn(), detach: vi.fn(), close: vi.fn() }
      }),
      openDecisionChat: vi.fn(),
    }
    await render({ initialView: 'overview', chat, iii: client(vi.fn()) })
    expect(requests).toHaveLength(towerFixture.items.filter((item) => item.kind !== 'stalled').length)
    const csv = requests.find((request) => request.questionId === 'csv')
    expect(csv).toBeDefined()
    await act(() => csv?.onUseAnswer(0, { text: 'Accept semicolons after the preview.' }))
    expect(container.textContent).toContain('Or answer in your own words')
    expect(
      container.querySelector<HTMLInputElement>('.tower-answer-field input:not([type="radio"]):not([type="checkbox"])')
        ?.value,
    ).toBe('Accept semicolons after the preview.')
    expect(chat.openDecisionChat).not.toHaveBeenCalled()
  })
})

describe('narrow pane and phone presentation', () => {
  const renderNarrow = async () =>
    act(() =>
      root.render(
        <TowerHost.Provider value={host}>
          <NeedsYou
            data={towerFixture}
            narrow
            user="user"
            initialSelectedId="security"
            onLeave={() => {}}
            openRuns={() => {}}
          />
        </TowerHost.Provider>,
      ),
    )

  it('shows inline detail in a narrow desktop pane instead of the host hidden sheet', async () => {
    await renderNarrow()
    expect(window.matchMedia).toHaveBeenCalledWith('(max-width: 767px)')
    expect(container.querySelector('.tower-needs-detail')).not.toBeNull()
    expect(container.querySelector('.tower-decision-sheet')).toBeNull()
    expect(container.querySelector('.tower-inbox')?.hasAttribute('data-phone')).toBe(false)
    expect(container.textContent).toContain(question.title)
  })

  it('starts with the phone list before the container measurement arrives', async () => {
    vi.mocked(window.matchMedia).mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList)
    const page = (narrow: boolean) => (
      <TowerHost.Provider value={host}>
        <NeedsYou data={towerFixture} narrow={narrow} user="user" onLeave={() => {}} openRuns={() => {}} />
      </TowerHost.Provider>
    )
    await act(() => root.render(page(false)))
    await act(() => root.render(page(true)))
    expect(container.querySelector('.tower-decision-sheet')).toBeNull()
    expect(container.querySelector('.tower-inbox')?.hasAttribute('data-detail-open')).toBe(false)
  })

  it('switches to the phone sheet at the host breakpoint and cleans up its listener', async () => {
    let change: () => void = () => {}
    const viewport = {
      matches: false,
      addEventListener: vi.fn((_event: string, listener: () => void) => {
        change = listener
      }),
      removeEventListener: vi.fn(),
    }
    vi.mocked(window.matchMedia).mockReturnValue(viewport as unknown as MediaQueryList)
    await renderNarrow()
    await act(() => {
      viewport.matches = true
      change()
    })
    expect(container.querySelector('.tower-decision-sheet')).not.toBeNull()
    expect(container.querySelector('.tower-needs-detail')).toBeNull()
    expect(container.querySelector('.tower-inbox')?.getAttribute('data-phone')).toBe('true')
    await act(() => {
      viewport.matches = false
      change()
    })
    expect(container.querySelector('.tower-needs-detail')).not.toBeNull()
    expect(container.querySelector('.tower-decision-sheet')).toBeNull()
    await act(() => root.render(null))
    expect(viewport.removeEventListener).toHaveBeenCalledWith('change', change)
  })
})
