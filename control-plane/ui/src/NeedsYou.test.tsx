// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider, host } from './test-ui'
vi.mock('@iii-dev/console-ui', async () => (await import('./test-ui')).mockUi)
import type { DecisionChatHandle, DecisionChatRequest, Host } from '@iii-dev/console-ui'
type IiiClient = Host['iii']
import { ControlPlaneScreen, type ControlPlaneScreenProps } from '.'
import { controlPlaneFixture } from './fixtures'
import { NeedsYou } from './NeedsYou'
import { ControlPlaneHost } from './host'
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
async function render(props: Partial<ControlPlaneScreenProps> = {}) {
  await act(() =>
    root.render(
      <TooltipProvider>
        <ControlPlaneScreen
          host={host}
          data={controlPlaneFixture}
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
const question = controlPlaneFixture.items[0] as QuestionNeed
const client = (call: ReturnType<typeof vi.fn>) => ({
  call: call as IiiClient['call'],
})

describe('Control plane Needs you interactions', () => {
  it('moves focus from Open to the decision and back to its list row on Escape', async () => {
    await render({ initialView: 'overview' })
    const open = button('Open')
    open.focus()
    await act(() => open.click()) // Native Enter activation on a button is a click.
    const heading = container.querySelector<HTMLHeadingElement>('.control-plane-decision h2')!
    expect(document.activeElement).toBe(heading)
    const row = container.querySelector<HTMLButtonElement>('.control-plane-need-row[aria-selected="true"]')
    await act(() => heading.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })))
    expect(container.querySelector('.control-plane-decision')).toBeNull()
    expect(document.activeElement).toBe(row)
  })

  it('replaces choices with the exact sent summary and sends only once on two clicks', async () => {
    let deliver: (entry: AskEntry) => void = () => {}
    const call = vi.fn(
      () =>
        new Promise<AskEntry>((resolve) => {
          deliver = resolve
        }),
    )
    const refresh = vi.fn()
    await render({ iii: client(call), data: { ...controlPlaneFixture, refresh } })
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
      by: { channel: 'control-plane', user: 'user' },
    })
    expect(container.textContent).toContain('Sending your answer')
    expect(container.querySelector('.control-plane-options')).toBeNull()
    await act(() =>
      deliver({
        ...question.entry,
        state: 'answered',
        answer_text: 'Builds: Isolate builds',
        by: { channel: 'control-plane', user: 'user' },
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
    expect(call.mock.calls[1]).toEqual(['control-plane::overview', { scope: 'all' }])
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
        ...controlPlaneFixture,
        items: controlPlaneFixture.items.filter((item) => item.id !== question.id),
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
              by: { channel: 'control-plane', user: 'user' },
            },
          },
        ],
      })
    await render({ iii: client(call) })
    await click('Answer')
    await click('Retry')
    expect(call.mock.calls.filter(([id]) => id === 'harness::ask::answer')).toHaveLength(1)
    expect(container.textContent).toContain('Sending your answer')
    expect(container.querySelector('.control-plane-options')).toBeNull()
  })
  it('lists releases beyond the Overview selection and filters their open items', async () => {
    const release = {
      ...controlPlaneFixture.releases[0],
      id: 'older',
      name: 'Older release',
    }
    const item = {
      ...controlPlaneFixture.items[0],
      card: { ...controlPlaneFixture.tickets[0], release_id: release.id },
    }
    await render({
      data: {
        ...controlPlaneFixture,
        releaseCatalog: [...controlPlaneFixture.releaseCatalog, release],
        items: [item],
      },
    })
    const rail = container.querySelector('.control-plane-needs-rail')
    const row = [...(rail?.querySelectorAll('button') ?? [])].find((node) =>
      node.textContent?.includes('Older release'),
    )
    expect(row).toBeDefined()
    await act(() => row?.click())
    expect(container.querySelectorAll('.control-plane-need-row')).toHaveLength(1)
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
      ...controlPlaneFixture,
      items: controlPlaneFixture.items.filter((item) => item.id !== question.id),
      questions: controlPlaneFixture.questions.map((row) =>
        row.questionId === question.questionId ? { ...row, state: settled.state, harness: settled } : row,
      ),
    }
    await render({ data })
    expect(container.textContent).toContain('Already answered by kanban')
    expect(container.querySelector('.control-plane-options')).toBeNull()
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
        ...controlPlaneFixture,
        questions: controlPlaneFixture.questions.map((row) => ({
          ...row,
          harness: undefined,
        })),
      },
    })
    expect(container.textContent).toContain('Expiry policy is unavailable until the harness returns.')
  })
  it('sends dismissed for scope questions and never offers it for security', async () => {
    const call = vi.fn().mockResolvedValue({
      ...(controlPlaneFixture.items.find((item) => item.id === 'csv') as QuestionNeed).entry,
      state: 'dismissed',
    })
    await render({ iii: client(call), initialSelectedId: 'csv' })
    await click('Let the agent decide')
    expect(call).toHaveBeenCalledWith('harness::ask::answer', {
      session_id: 'import-tool',
      question_id: 'csv',
      dismissed: true,
      by: { channel: 'control-plane', user: 'user' },
    })
  })

  it('shows a dismissed decision as left to the agent without the delivery instruction', async () => {
    const instruction = 'I dismissed your questions without answering them. Go on with your best judgement.'
    await render({
      initialView: 'decided',
      data: {
        ...controlPlaneFixture,
        questions: controlPlaneFixture.questions.map((row) =>
          row.recordedBy ? row : { ...row, state: 'dismissed', outcome: 'dismissed', answerText: instruction },
        ),
      },
    })
    expect(container.textContent).toContain('Left to the agent')
    expect(container.textContent).not.toContain(instruction)
  })
  it('reuses the Resume idempotency key across panes and changes it for a newer stalled turn', async () => {
    const call = vi.fn().mockResolvedValue({})
    for (const turnId of ['first-turn', 'first-turn', 'second-turn']) {
      await act(() => root.unmount())
      root = createRoot(container)
      await render({
        iii: client(call),
        initialSelectedId: 'stall',
        data: {
          ...controlPlaneFixture,
          runs: controlPlaneFixture.runs.map((run) =>
            run.session.session_id === 'dispatch-flaky'
              ? { ...run, status: { ...run.status, turn_id: turnId } }
              : run,
          ),
        },
      })
      await click('Resume')
    }
    expect(call).toHaveBeenCalledTimes(3)
    expect(call).toHaveBeenNthCalledWith(1, 'harness::send', {
      session_id: 'dispatch-flaky',
      message: 'continue',
      idempotency_key: expect.stringMatching(/^control-plane:resume:dispatch-flaky:.*first-turn/),
    })
    expect(call.mock.calls[1]).toEqual(call.mock.calls[0])
    expect(call.mock.calls[2][1].idempotency_key).not.toEqual(call.mock.calls[0][1].idempotency_key)
  })
  it('leaves a stalled item hidden until its run changes', async () => {
    await render({ initialSelectedId: 'stall' })
    await click('Leave it')
    expect(container.querySelectorAll('.control-plane-need-row')).toHaveLength(controlPlaneFixture.items.length - 1)
    await render({
      data: {
        ...controlPlaneFixture,
        runs: controlPlaneFixture.runs.map((run) =>
          run.session.session_id === 'dispatch-flaky'
            ? { ...run, status: { ...run.status, turn_id: 'new-turn' } }
            : run,
        ),
      },
    })
    expect(container.querySelectorAll('.control-plane-need-row')).toHaveLength(controlPlaneFixture.items.length)
  })
})
describe('Control plane question logs', () => {
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
  it('lists closed questions without recorded assumptions', async () => {
    await render({ initialView: 'decided' })
    expect(container.textContent).toContain('May builds reach the public network?')
    expect(container.textContent).toContain('Allow only the package mirrors.')
    expect(container.textContent).toContain('not delivered')
    expect(button('Open card')).toBeDefined()
    expect(container.textContent).not.toContain('Cache artifacts between builds?')
  })
})

describe('Control plane decision capability', () => {
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
          <ControlPlaneScreen
            host={host}
            data={controlPlaneFixture}
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
          <ControlPlaneScreen
            host={host}
            data={controlPlaneFixture}
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
      controlPlaneFixture.items.filter((item) => item.kind !== 'stalled').length,
    )
    await act(() =>
      root.render(
        <TooltipProvider>
          <ControlPlaneScreen
            host={host}
            data={{ ...controlPlaneFixture, items: controlPlaneFixture.items.slice(0, 1) }}
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
    await act(() => new Promise((resolve) => setTimeout(resolve, 0)))
    expect(bindings[1].detach).toHaveBeenCalledOnce()
    await act(() => root.unmount())
    expect(bindings[0].detach).toHaveBeenCalled()
    expect(bindings[1].detach).toHaveBeenCalledOnce()
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
          <ControlPlaneScreen
            host={host}
            data={controlPlaneFixture}
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
          <ControlPlaneScreen host={host} data={controlPlaneFixture} scope="all" onScopeChange={() => {}} initialView="needs-you" />
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
    expect(requests).toHaveLength(controlPlaneFixture.items.filter((item) => item.kind !== 'stalled').length)
    const csv = requests.find((request) => request.questionId === 'csv')
    expect(csv).toBeDefined()
    await act(() => csv?.onUseAnswer(0, { text: 'Accept semicolons after the preview.' }))
    expect(container.textContent).toContain('Or answer in your own words')
    expect(
      container.querySelector<HTMLInputElement>('.control-plane-answer-field input:not([type="radio"]):not([type="checkbox"])')
        ?.value,
    ).toBe('Accept semicolons after the preview.')
    expect(chat.openDecisionChat).not.toHaveBeenCalled()
  })
})

describe('narrow pane and phone presentation', () => {
  const renderNarrow = async () =>
    act(() =>
      root.render(
        <ControlPlaneHost.Provider value={host}>
          <NeedsYou
            data={controlPlaneFixture}
            narrow
            user="user"
            initialSelectedId="security"
            onLeave={() => {}}
            openRuns={() => {}}
          />
        </ControlPlaneHost.Provider>,
      ),
    )

  it('shows inline detail in a narrow desktop pane instead of the host hidden sheet', async () => {
    await renderNarrow()
    expect(window.matchMedia).toHaveBeenCalledWith('(max-width: 767px)')
    expect(container.querySelector('.control-plane-needs-detail')).not.toBeNull()
    expect(container.querySelector('.control-plane-decision-sheet')).toBeNull()
    expect(container.querySelector('.control-plane-inbox')?.hasAttribute('data-phone')).toBe(false)
    expect(container.textContent).toContain(question.title)
  })

  it('starts with the phone list before the container measurement arrives', async () => {
    vi.mocked(window.matchMedia).mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList)
    const page = (narrow: boolean) => (
      <ControlPlaneHost.Provider value={host}>
        <NeedsYou data={controlPlaneFixture} narrow={narrow} user="user" onLeave={() => {}} openRuns={() => {}} />
      </ControlPlaneHost.Provider>
    )
    await act(() => root.render(page(false)))
    await act(() => root.render(page(true)))
    expect(container.querySelector('.control-plane-decision-sheet')).toBeNull()
    expect(container.querySelector('.control-plane-inbox')?.hasAttribute('data-detail-open')).toBe(false)
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
    expect(container.querySelector('.control-plane-decision-sheet')).not.toBeNull()
    expect(container.querySelector('.control-plane-needs-detail')).toBeNull()
    expect(container.querySelector('.control-plane-inbox')?.getAttribute('data-phone')).toBe('true')
    await act(() => {
      viewport.matches = false
      change()
    })
    expect(container.querySelector('.control-plane-needs-detail')).not.toBeNull()
    expect(container.querySelector('.control-plane-decision-sheet')).toBeNull()
    await act(() => root.render(null))
    expect(viewport.removeEventListener).toHaveBeenCalledWith('change', change)
  })
})
