// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { AutocompactMark, SnapshotAutocompact } from '@/lib/autocompact'
import { useTurnAutocompact } from './use-turn-autocompact'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const GLOBAL_400K: AutocompactMark = {
  effective: 400_000,
  rule: { scope: 'global' },
}
const PROJECT_160K: AutocompactMark = {
  effective: 160_000,
  rule: { scope: 'project', project_id: 'p-acme', project_name: 'acme-web' },
}
const SNAPSHOT_600K: SnapshotAutocompact = {
  setting_tokens: 600_000,
  effective_tokens: 600_000,
  cap_tokens: 660_000,
  rule: { scope: 'session' },
}

let root: Root
let host: HTMLDivElement
let shown: AutocompactMark | null = null
/** The observers a turn opened: their session, their callback, closed or not. */
let observers: {
  sessionId: string
  onValue: (value: SnapshotAutocompact) => void
  closed: boolean
}[] = []
const observe = vi.fn(
  (sessionId: string, onValue: (value: SnapshotAutocompact) => void) => {
    const observer = { sessionId, onValue, closed: false }
    observers.push(observer)
    return () => {
      observer.closed = true
    }
  },
)

function Probe(props: {
  sessionId: string
  running: boolean
  between: AutocompactMark | null
}) {
  shown = useTurnAutocompact({ ...props, observe })
  return null
}

function show(
  between: AutocompactMark | null,
  running: boolean,
  sessionId = 's1',
) {
  act(() => {
    root.render(
      <Probe sessionId={sessionId} running={running} between={between} />,
    )
  })
}

beforeEach(() => {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  observers = []
  observe.mockClear()
  shown = null
})

afterEach(() => {
  act(() => root.unmount())
  host.remove()
})

describe('useTurnAutocompact (§3.3.3)', () => {
  it('shows the shared resolution between turns, live (D164)', () => {
    show(GLOBAL_400K, false)
    expect(shown).toEqual(GLOBAL_400K)
    show(PROJECT_160K, false)
    expect(shown).toEqual(PROJECT_160K)
    // Nothing observes between turns.
    expect(observe).not.toHaveBeenCalled()
  })

  it('keeps what the turn started with while it runs', () => {
    show(GLOBAL_400K, false)
    show(GLOBAL_400K, true)
    expect(shown).toEqual(GLOBAL_400K)
    // A setting changed during the turn applies at its end, as the harness
    // applies it then (finalize resolves again).
    show(PROJECT_160K, true)
    expect(shown).toEqual(GLOBAL_400K)
    show(PROJECT_160K, false)
    expect(shown).toEqual(PROJECT_160K)
  })

  it("reads the turn's own snapshot once a step writes one", () => {
    show(GLOBAL_400K, false)
    show(GLOBAL_400K, true)
    expect(observers.map((o) => o.sessionId)).toEqual(['s1'])
    act(() => observers[0]?.onValue(SNAPSHOT_600K))
    expect(shown).toEqual({ effective: 600_000, rule: { scope: 'session' } })
    // Past the model's limit the snapshot says null: no tick, no chip.
    act(() =>
      observers[0]?.onValue({
        ...SNAPSHOT_600K,
        effective_tokens: null,
      }),
    )
    expect(shown).toEqual({ effective: null, rule: { scope: 'session' } })
  })

  it('drops the turn when it ends, and stops observing', () => {
    show(GLOBAL_400K, true)
    act(() => observers[0]?.onValue(SNAPSHOT_600K))
    show(GLOBAL_400K, false)
    expect(observers[0]?.closed).toBe(true)
    expect(shown).toEqual(GLOBAL_400K)
    // A late write of the ended turn changes nothing.
    act(() => observers[0]?.onValue(SNAPSHOT_600K))
    expect(shown).toEqual(GLOBAL_400K)
    // The next turn starts from the resolution again, not the old snapshot.
    show(PROJECT_160K, true)
    expect(shown).toEqual(PROJECT_160K)
  })

  it('starts over for another conversation', () => {
    show(GLOBAL_400K, true, 's1')
    act(() => observers[0]?.onValue(SNAPSHOT_600K))
    show(PROJECT_160K, true, 's2')
    expect(shown).toEqual(PROJECT_160K)
    expect(observers[0]?.closed).toBe(true)
    expect(observers.at(-1)?.sessionId).toBe('s2')
  })

  it('freezes the resolution when a backend has no snapshot to observe', () => {
    function Bare(props: { running: boolean; between: AutocompactMark }) {
      shown = useTurnAutocompact({ sessionId: 's1', ...props })
      return null
    }
    act(() => root.render(<Bare running between={GLOBAL_400K} />))
    act(() => root.render(<Bare running between={PROJECT_160K} />))
    expect(shown).toEqual(GLOBAL_400K)
  })
})
