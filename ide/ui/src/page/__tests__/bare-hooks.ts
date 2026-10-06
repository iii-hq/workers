/* One component's hooks, run bare in node: ide/ui has no DOM and no test
   renderer, and a hook's timing (what an effect reads, when it re-reads)
   is what its tests check. `vi.mock('react', ...)` swaps these in.

   Hooks keep their slots by call order. A state update renders again at
   once, or right after the render or effects in progress; one made while
   rendering re-runs the render before any effect, as React does. Effects
   run after a render whose dependencies changed, their cleanups before
   the next run and on unmount. */

type Deps = readonly unknown[] | undefined

interface Slot {
  ready?: boolean
  value?: unknown
  deps?: Deps
  cleanup?: unknown
  set?: (next: unknown) => void
}

interface Instance {
  slots: Slot[]
  at: number
  effects: Array<() => void>
  dirty: boolean
  busy: boolean
  dead: boolean
  render(): void
}

let current: Instance | null = null

function slot(): [Slot, Instance] {
  const instance = current
  if (instance === null) throw new Error('a hook called outside mount()')
  instance.slots[instance.at] ??= {}
  return [instance.slots[instance.at++], instance]
}

const moved = (last: Deps, next: Deps) =>
  last === undefined || next === undefined || last.some((value, at) => !Object.is(value, next[at]))

function useMemo<T>(make: () => T, deps: Deps): T {
  const [s] = slot()
  if (!s.ready || moved(s.deps, deps)) {
    s.value = make()
    s.deps = deps
    s.ready = true
  }
  return s.value as T
}

export const hooks = {
  useState<T>(initial: T | (() => T)) {
    const [s, instance] = slot()
    if (!s.ready) {
      s.value = typeof initial === 'function' ? (initial as () => T)() : initial
      s.ready = true
    }
    s.set ??= (next) => {
      const value = typeof next === 'function' ? (next as (last: unknown) => unknown)(s.value) : next
      if (instance.dead || Object.is(value, s.value)) return
      s.value = value
      instance.dirty = true
      if (!instance.busy) instance.render()
    }
    return [s.value as T, s.set]
  },
  useRef<T>(initial: T) {
    return useMemo(() => ({ current: initial }), [])
  },
  useMemo,
  useCallback<T>(fn: T, deps: Deps) {
    return useMemo(() => fn, deps)
  },
  useEffect(effect: () => unknown, deps: Deps) {
    const [s, instance] = slot()
    if (s.ready && !moved(s.deps, deps)) return
    instance.effects.push(() => {
      if (typeof s.cleanup === 'function') s.cleanup()
      s.cleanup = effect()
      s.deps = deps
      s.ready = true
    })
  },
}

/** Renders `component` with `props` and runs its effects. */
export function mount<P, R>(component: (props: P) => R, props: NoInfer<P>) {
  let result: R | undefined
  const instance: Instance = {
    slots: [],
    at: 0,
    effects: [],
    dirty: true,
    busy: false,
    dead: false,
    render() {
      instance.busy = true
      try {
        while (instance.dirty && !instance.dead) {
          instance.dirty = false
          instance.effects = []
          const outer = current
          current = instance
          instance.at = 0
          try {
            result = component(props)
          } finally {
            current = outer
          }
          if (instance.dirty) continue
          for (const run of instance.effects.splice(0)) run()
        }
      } finally {
        instance.busy = false
      }
    },
  }
  instance.render()
  return {
    get result() {
      return result as R
    },
    rerender(next: P) {
      props = next
      instance.dirty = true
      instance.render()
    },
    unmount() {
      instance.dead = true
      for (const s of instance.slots) if (typeof s.cleanup === 'function') s.cleanup()
    },
  }
}
