/**
 * Test double of the disposer `IiiClient.observe` returns, with a state the
 * test moves the way the engine would (`set('active')`, `set('suspended')`).
 * `ready()` resolves once it is active, like the SDK's.
 */

import type { ObserverDisposer } from '@/lib/iii-client'

type State = ObserverDisposer['state']

export interface FakeObserver {
  observer: ObserverDisposer
  set(state: State): void
  readonly disposed: boolean
  readonly listeners: number
}

export function fakeObserver(
  id = 'obs',
  initial: State = 'pending',
  onDispose?: () => void,
): FakeObserver {
  let state = initial
  let disposed = false
  const listeners = new Set<(state: State) => void>()
  let waiters: Array<() => void> = []
  const dispose = () => {
    disposed = true
    onDispose?.()
  }
  const observer = Object.defineProperties(dispose, {
    id: { value: id },
    state: { get: () => state },
    reason: { get: () => undefined },
    ready: {
      value: () =>
        state === 'active'
          ? Promise.resolve()
          : new Promise<void>((resolve) => waiters.push(resolve)),
    },
    onStateChange: {
      value: (listener: (state: State) => void) => {
        listeners.add(listener)
        return () => {
          listeners.delete(listener)
        }
      },
    },
  }) as ObserverDisposer
  return {
    observer,
    set(next) {
      state = next
      if (next === 'active') {
        const ready = waiters
        waiters = []
        for (const resolve of ready) resolve()
      }
      for (const listener of [...listeners]) listener(next)
    },
    get disposed() {
      return disposed
    },
    get listeners() {
      return listeners.size
    },
  }
}
