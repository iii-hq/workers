// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import { clearTurnStartsForTests, turnStartedAt } from '@/lib/turn-starts'
import {
  rawSplits,
  rawUsage,
} from '@/stories/fixtures/claude-accounts-fixtures'
import {
  ACCOUNT_SPLIT_FUNCTION_ID,
  getClaudeAccountsState,
  RETRY_FUNCTION_ID,
  refreshClaudeAccounts,
  retryTurn,
  seedClaudeAccountsForTests,
  TMUX_CHANGED_FUNCTION_ID,
  USAGE_READ_FUNCTION_ID,
  useClaudeAccounts,
} from './claude-accounts'

const NOW = Date.UTC(2026, 10, 21, 10, 0)

/** A held answer: the test says when tmux answers. */
interface Pending {
  refresh: boolean
  answer: () => void
}

const engine = vi.hoisted(() => ({
  call: vi.fn<(id: string, payload: Record<string, unknown>) => unknown>(),
  on: vi.fn(),
  observe: vi.fn(),
  offOn: vi.fn(),
  offObserve: vi.fn(),
}))

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () =>
    Promise.resolve({
      browserId: 'browser-1',
      call: engine.call,
      on: (...args: unknown[]) => {
        engine.on(...args)
        return engine.offOn
      },
      observe: (...args: unknown[]) => {
        engine.observe(...args)
        return engine.offObserve
      },
    }),
}))

let readings = 0
let held: Pending[] = []
let holdUsage = false

function Probe({ splits = false }: { splits?: boolean }) {
  useClaudeAccounts(true, { splits })
  return null
}

const roots: Root[] = []
function mount(splits = false): Root {
  const root = createRoot(
    document.body.appendChild(document.createElement('div')),
  )
  act(() => root.render(<Probe splits={splits} />))
  roots.push(root)
  return root
}

/** Let the client promise, the reading and the watch settle. */
async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve()
  })
}

const calls = (id: string) => engine.call.mock.calls.filter(([fn]) => fn === id)

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(NOW)
  readings = 0
  held = []
  holdUsage = false
  engine.call.mockReset()
  engine.call.mockImplementation((id, payload) => {
    if (id === ACCOUNT_SPLIT_FUNCTION_ID) return Promise.resolve(rawSplits(NOW))
    if (id !== USAGE_READ_FUNCTION_ID) return Promise.resolve({})
    // Each reading is newer than the last.
    const reading = rawUsage(NOW + ++readings * 1000)
    if (!holdUsage) return Promise.resolve(reading)
    return new Promise((resolve) =>
      held.push({
        refresh: payload.refresh === true,
        answer: () => resolve(reading),
      }),
    )
  })
  for (const fn of [engine.on, engine.observe, engine.offOn, engine.offObserve])
    fn.mockReset()
  setChatCapabilitiesForTests({
    harnessRetry: true,
    tmuxAccounts: true,
    tmuxCredential: true,
    patchMeta: true,
  })
})

afterEach(() => {
  for (const root of roots.splice(0)) act(() => root.unmount())
  document.body.innerHTML = ''
  seedClaudeAccountsForTests(null)
  setChatCapabilitiesForTests(null)
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('the accounts store', () => {
  it('observes tmux while a view holds it, and stops when the last one goes', async () => {
    const first = mount()
    const second = mount()
    await settle()
    expect(engine.observe).toHaveBeenCalledTimes(1)
    expect(engine.observe.mock.calls[0][0]).toBe(TMUX_CHANGED_FUNCTION_ID)
    expect(calls(USAGE_READ_FUNCTION_ID)).toHaveLength(1)
    expect(getClaudeAccountsState().usage).not.toBeNull()

    act(() => first.unmount())
    expect(engine.offObserve).not.toHaveBeenCalled()
    act(() => second.unmount())
    roots.length = 0
    expect(engine.offObserve).toHaveBeenCalledTimes(1)
    expect(engine.offOn).toHaveBeenCalledTimes(1)
    // The last reading stays for the next view…
    expect(getClaudeAccountsState().usage).not.toBeNull()
    // …which reads and observes again.
    mount()
    await settle()
    expect(engine.observe).toHaveBeenCalledTimes(2)
    expect(calls(USAGE_READ_FUNCTION_ID)).toHaveLength(2)
  })

  it('stops "Refreshing" when the manual read answers, whatever read came between', async () => {
    mount()
    await settle()
    holdUsage = true
    const manual = refreshClaudeAccounts(true)
    expect(getClaudeAccountsState().refreshing).toBe(true)
    const pushed = refreshClaudeAccounts()
    await settle()
    expect(held.map((p) => p.refresh)).toEqual([true, false])
    held[1].answer()
    await act(() => pushed)
    expect(getClaudeAccountsState().refreshing).toBe(true)
    held[0].answer()
    await act(() => manual)
    expect(getClaudeAccountsState().refreshing).toBe(false)
  })

  it('reads the split on a manual refresh at most once a minute', async () => {
    mount(true)
    await settle()
    expect(calls(ACCOUNT_SPLIT_FUNCTION_ID)).toHaveLength(1)
    await act(() => refreshClaudeAccounts(true))
    await act(() => refreshClaudeAccounts(true))
    expect(calls(USAGE_READ_FUNCTION_ID)).toHaveLength(3)
    expect(calls(ACCOUNT_SPLIT_FUNCTION_ID)).toHaveLength(1)
    vi.setSystemTime(NOW + 61_000)
    await act(() => refreshClaudeAccounts(true))
    expect(calls(ACCOUNT_SPLIT_FUNCTION_ID)).toHaveLength(2)
  })

  it('notes when the re-run turn it asked for started, for its reply’s duration', async () => {
    engine.call.mockImplementation((id) =>
      Promise.resolve(id === RETRY_FUNCTION_ID ? { turn_id: 't_9' } : {}),
    )
    clearTurnStartsForTests()
    await expect(retryTurn('s1')).resolves.toEqual({ turnId: 't_9' })
    expect(engine.call).toHaveBeenCalledWith(RETRY_FUNCTION_ID, {
      session_id: 's1',
    })
    expect(turnStartedAt('t_9')).toBe(NOW)
  })
})
