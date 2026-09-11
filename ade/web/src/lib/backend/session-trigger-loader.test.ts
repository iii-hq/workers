import { describe, expect, it, vi } from 'vitest'
import { startSessionTriggerLoader } from './session-trigger-loader'
import type { SessionTriggerInfo } from './triggers'

const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

function trigger(id: string): SessionTriggerInfo {
  return {
    id,
    triggerType: 'state',
    delivery: { kind: 'notify' },
  }
}

function setup(
  listTriggers: (sessionId: string) => Promise<SessionTriggerInfo[]>,
) {
  let notifyChanged = () => {}
  const unsubscribe = vi.fn()
  const onTriggersChanged = vi.fn((_sessionId: string, onEvent: () => void) => {
    notifyChanged = onEvent
    return unsubscribe
  })
  const onSnapshot = vi.fn()
  const loader = startSessionTriggerLoader({
    sessionId: 'session-1',
    listTriggers,
    onTriggersChanged,
    onSnapshot,
  })

  return {
    loader,
    notifyChanged: () => notifyChanged(),
    onSnapshot,
    onTriggersChanged,
    unsubscribe,
  }
}

describe('startSessionTriggerLoader', () => {
  it('uses one initial read and one listener for the shared snapshot', async () => {
    const rows = [trigger('sub-1'), trigger('sub-2'), trigger('sub-3')]
    const listTriggers = vi.fn(async () => rows)
    const state = setup(listTriggers)

    expect(state.onTriggersChanged).toHaveBeenCalledTimes(1)
    expect(state.onTriggersChanged).toHaveBeenCalledWith(
      'session-1',
      expect.any(Function),
    )
    expect(listTriggers).toHaveBeenCalledTimes(1)
    expect(state.onTriggersChanged.mock.invocationCallOrder[0]).toBeLessThan(
      listTriggers.mock.invocationCallOrder[0],
    )

    await flush()
    expect(state.onSnapshot).toHaveBeenCalledOnce()
    expect(state.onSnapshot).toHaveBeenCalledWith(rows)

    state.notifyChanged()
    await flush()
    expect(listTriggers).toHaveBeenCalledTimes(2)
    expect(state.onTriggersChanged).toHaveBeenCalledTimes(1)

    state.loader.dispose()
    state.notifyChanged()
    await flush()
    expect(state.unsubscribe).toHaveBeenCalledOnce()
    expect(listTriggers).toHaveBeenCalledTimes(2)
  })

  it('applies only successful reads and keeps the last snapshot on failure', async () => {
    const initial = [trigger('sub-active')]
    const recovered = [trigger('sub-active'), trigger('sub-new')]
    const listTriggers = vi
      .fn<(sessionId: string) => Promise<SessionTriggerInfo[]>>()
      .mockRejectedValueOnce(new Error('initially unavailable'))
      .mockResolvedValueOnce(initial)
      .mockRejectedValueOnce(new Error('temporarily unavailable'))
      .mockResolvedValueOnce(recovered)
    const state = setup(listTriggers)

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
