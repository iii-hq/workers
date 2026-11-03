import { describe, expect, it, vi } from 'vitest'
import type { SessionObserverInfo } from './observers'
import { startSessionObserverLoader } from './session-observer-loader'

const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

function observer(id: string): SessionObserverInfo {
  return {
    id,
    functionId: 'state::on-changed',
    delivery: { kind: 'notify' },
  }
}

function setup(
  listObservers: (sessionId: string) => Promise<SessionObserverInfo[]>,
) {
  let notifyChanged = () => {}
  const unsubscribe = vi.fn()
  const onObserversChanged = vi.fn(
    (_sessionId: string, onEvent: () => void) => {
      notifyChanged = onEvent
      return unsubscribe
    },
  )
  const onSnapshot = vi.fn()
  const loader = startSessionObserverLoader({
    sessionId: 'session-1',
    listObservers,
    onObserversChanged,
    onSnapshot,
  })

  return {
    loader,
    notifyChanged: () => notifyChanged(),
    onSnapshot,
    onObserversChanged,
    unsubscribe,
  }
}

describe('startSessionObserverLoader', () => {
  it('uses one initial read and one listener for the shared snapshot', async () => {
    const rows = [observer('obs-1'), observer('obs-2'), observer('obs-3')]
    const listObservers = vi.fn(async () => rows)
    const state = setup(listObservers)

    expect(state.onObserversChanged).toHaveBeenCalledTimes(1)
    expect(state.onObserversChanged).toHaveBeenCalledWith(
      'session-1',
      expect.any(Function),
    )
    expect(listObservers).toHaveBeenCalledTimes(1)
    expect(state.onObserversChanged.mock.invocationCallOrder[0]).toBeLessThan(
      listObservers.mock.invocationCallOrder[0],
    )

    await flush()
    expect(state.onSnapshot).toHaveBeenCalledOnce()
    expect(state.onSnapshot).toHaveBeenCalledWith(rows)

    state.notifyChanged()
    await flush()
    expect(listObservers).toHaveBeenCalledTimes(2)
    expect(state.onObserversChanged).toHaveBeenCalledTimes(1)

    state.loader.dispose()
    state.notifyChanged()
    await flush()
    expect(state.unsubscribe).toHaveBeenCalledOnce()
    expect(listObservers).toHaveBeenCalledTimes(2)
  })

  it('applies only successful reads and keeps the last snapshot on failure', async () => {
    const initial = [observer('obs-active')]
    const recovered = [observer('obs-active'), observer('obs-new')]
    const listObservers = vi
      .fn<(sessionId: string) => Promise<SessionObserverInfo[]>>()
      .mockRejectedValueOnce(new Error('initially unavailable'))
      .mockResolvedValueOnce(initial)
      .mockRejectedValueOnce(new Error('temporarily unavailable'))
      .mockResolvedValueOnce(recovered)
    const state = setup(listObservers)

    await flush()
    expect(state.onSnapshot).not.toHaveBeenCalled()

    state.notifyChanged()
    await flush()
    expect(state.onSnapshot.mock.calls).toEqual([[initial]])

    state.notifyChanged()
    await flush()
    expect(state.onSnapshot.mock.calls).toEqual([[initial]])

    state.notifyChanged()
    await flush()
    expect(state.onSnapshot.mock.calls).toEqual([[initial], [recovered]])

    state.loader.dispose()
  })
})
