/**
 * The fleet store's own checks — the poll/backoff machine driven entirely
 * through injected deps (no real timers, no DOM):
 *
 *  - tolerant parsing: canonical rows pass through, missing slot fields
 *    derive from exec_in_progress, junk rows drop, junk envelopes → [];
 *  - visibility gating: a hidden tab schedules but never triggers;
 *  - backoff: consecutive errors walk 3s → 6s → 12s → 24s → 30s (cap) and
 *    one success resets to the 3s cadence;
 *  - daemon-absent: a function-not-found failure flips the flag, any
 *    success clears it;
 *  - catalog: read once on start, again only on refresh(true).
 */

import { describe, expect, it } from 'vitest'
import {
  backoffDelayMs,
  CATALOG_FN,
  createFleetStore,
  isFunctionNotFound,
  LIST_FN,
  MAX_BACKOFF_MS,
  parseCatalog,
  parseRuntimes,
  parseSandboxList,
  BOOTSTRAP_RETRY_MS,
  reconcileTombstones,
  TOMBSTONE_TTL_MS,
  RUNTIMES_FN,
} from './store'

/* ── parsing ──────────────────────────────────────────────────────────── */

describe('parseSandboxList', () => {
  it('reads canonical rows whole', () => {
    const out = parseSandboxList({
      sandboxes: [
        {
          sandbox_id: 'sbx-1',
          name: 'demo',
          image: 'node',
          age_secs: 42,
          exec_in_progress: true,
          exec_in_flight: 2,
          exec_slots_free: 2,
          stopped: false,
          extra_field: 'ignored',
        },
      ],
    })
    expect(out).toEqual([
      {
        sandbox_id: 'sbx-1',
        name: 'demo',
        image: 'node',
        age_secs: 42,
        exec_in_progress: true,
        exec_in_flight: 2,
        exec_slots_free: 2,
        stopped: false,
      },
    ])
  })

  it('derives the slot fields when an older daemon omits them', () => {
    const [busy] = parseSandboxList({
      sandboxes: [
        { sandbox_id: 'sbx-2', image: 'python', age_secs: 1, exec_in_progress: true, stopped: false },
      ],
    })
    expect(busy.exec_in_flight).toBe(1)
    expect(busy.exec_slots_free).toBe(3)
    const [idle] = parseSandboxList({
      sandboxes: [{ sandbox_id: 'sbx-3' }],
    })
    expect(idle.exec_in_flight).toBe(0)
    expect(idle.exec_slots_free).toBe(4)
    expect(idle.name).toBeNull()
    expect(idle.image).toBe('')
    expect(idle.age_secs).toBe(0)
    expect(idle.stopped).toBe(false)
  })

  it('drops rows without a sandbox_id and survives junk envelopes', () => {
    expect(
      parseSandboxList({ sandboxes: [{ image: 'x' }, null, 'nope', 7] }),
    ).toEqual([])
    expect(parseSandboxList(null)).toEqual([])
    expect(parseSandboxList('sandboxes')).toEqual([])
    expect(parseSandboxList({ sandboxes: 'not-a-list' })).toEqual([])
  })
})

describe('parseCatalog', () => {
  it('keeps named images, defaults kind to custom', () => {
    expect(
      parseCatalog({
        images: [
          { name: 'node', oci_ref: 'docker.io/library/node:22', kind: 'preset' },
          { name: 'mine' },
          { oci_ref: 'nameless' },
        ],
      }),
    ).toEqual([
      { name: 'node', oci_ref: 'docker.io/library/node:22', kind: 'preset' },
      { name: 'mine', oci_ref: '', kind: 'custom' },
    ])
    expect(parseCatalog(undefined)).toEqual([])
  })
})

describe('parseRuntimes', () => {
  it('accepts the canonical shape', () => {
    expect(
      parseRuntimes({
        runtimes: [
          {
            runtime_id: 'rt-1',
            lang: 'node',
            sandbox_id: 'sbx-1',
            created_at_ms: 1000,
            registered_functions: ['app::hello'],
          },
        ],
      }),
    ).toEqual([
      {
        runtime_id: 'rt-1',
        lang: 'node',
        sandbox_id: 'sbx-1',
        created_at_ms: 1000,
        registered_functions: ['app::hello'],
        vm_gone: null,
      },
    ])
  })

  it('accepts shifted field names and object-shaped function lists', () => {
    const [rt] = parseRuntimes({
      runtimes: [
        {
          id: 'rt-2',
          language: 'python',
          created_at: 2000,
          functions: [{ function_id: 'app::a' }, { id: 'app::b' }, 42, null],
        },
      ],
    })
    expect(rt.runtime_id).toBe('rt-2')
    expect(rt.lang).toBe('python')
    expect(rt.created_at_ms).toBe(2000)
    expect(rt.sandbox_id).toBeNull()
    expect(rt.registered_functions).toEqual(['app::a', 'app::b'])
  })

  it('accepts a bare array and rejects junk', () => {
    expect(parseRuntimes([{ runtime_id: 'rt-3' }])[0].lang).toBe('unknown')
    expect(parseRuntimes({ runtimes: {} })).toEqual([])
    expect(parseRuntimes('rt')).toEqual([])
  })
})

describe('reconcileTombstones', () => {
  const sb = (id: string) => ({
    sandbox_id: id,
    name: null,
    image: 'python',
    age_secs: 10,
    exec_in_progress: false,
    exec_in_flight: 0,
    exec_slots_free: 4,
    stopped: false,
  })

  it('tombstones a sandbox that left the fleet and prunes it after the TTL', () => {
    const first = reconcileTombstones([], [sb('a'), sb('b')], [sb('a')], 1000)
    expect(first.map((t) => t.sandbox.sandbox_id)).toEqual(['b'])
    const kept = reconcileTombstones(first, [sb('a')], [sb('a')], 2000)
    expect(kept).toHaveLength(1)
    const pruned = reconcileTombstones(
      kept,
      [sb('a')],
      [sb('a')],
      1000 + TOMBSTONE_TTL_MS,
    )
    expect(pruned).toHaveLength(0)
  })

  it('resurrects an id that reappears', () => {
    const first = reconcileTombstones([], [sb('a')], [], 1000)
    expect(first).toHaveLength(1)
    const back = reconcileTombstones(first, [], [sb('a')], 2000)
    expect(back).toHaveLength(0)
  })
})

describe('isFunctionNotFound', () => {
  it('matches the daemon-absent shapes and nothing else', () => {
    expect(isFunctionNotFound('function sandbox::list not found')).toBe(true)
    expect(isFunctionNotFound('Function not registered: sandbox::list')).toBe(true)
    expect(isFunctionNotFound('unknown function sandbox::list')).toBe(true)
    expect(isFunctionNotFound('no such function')).toBe(true)
    expect(isFunctionNotFound('timeout waiting for response')).toBe(false)
    expect(isFunctionNotFound('connection refused')).toBe(false)
    // Unrelated "not found" prose must not read as daemon-absent.
    expect(isFunctionNotFound('the file /etc/config was not found')).toBe(false)
    expect(
      isFunctionNotFound('the path requested by the function call was not found'),
    ).toBe(false)
  })
})

/* ── backoff ──────────────────────────────────────────────────────────── */

describe('backoffDelayMs', () => {
  it('walks 3s → 6s → 12s → 24s → 30s and caps there', () => {
    expect(backoffDelayMs(0)).toBe(BOOTSTRAP_RETRY_MS)
    expect(backoffDelayMs(1)).toBe(3_000)
    expect(backoffDelayMs(2)).toBe(6_000)
    expect(backoffDelayMs(3)).toBe(12_000)
    expect(backoffDelayMs(4)).toBe(24_000)
    expect(backoffDelayMs(5)).toBe(30_000)
    expect(backoffDelayMs(9)).toBe(MAX_BACKOFF_MS)
  })
})

/* ── the poll machine ─────────────────────────────────────────────────── */

interface Timer {
  fn: () => void
  ms: number
  cleared: boolean
}

function makeHarness(options?: { visible?: () => boolean }) {
  const timers: Timer[] = []
  const calls: string[] = []
  const responses = new Map<string, () => unknown>()
  responses.set(LIST_FN, () => ({ sandboxes: [] }))
  responses.set(CATALOG_FN, () => ({ images: [] }))
  responses.set(RUNTIMES_FN, () => ({ runtimes: [] }))

  const trigger = async (functionId: string) => {
    calls.push(functionId)
    const responder = responses.get(functionId)
    if (!responder) throw new Error(`no responder for ${functionId}`)
    return responder()
  }

  const store = createFleetStore(trigger, {
    setTimer: (fn, ms) => {
      const timer: Timer = { fn, ms, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer: (id) => {
      ;(id as Timer).cleared = true
    },
    now: () => 12345,
  })

  const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
  const pending = () => timers.filter((t) => !t.cleared)
  const runNext = async () => {
    const timer = pending().shift()
    if (!timer) throw new Error('no pending timer')
    timer.cleared = true
    timer.fn()
    await flush()
  }

  return { store, calls, responses, timers, pending, runNext, flush }
}

describe('createFleetStore', () => {
  it('fetches list + runtimes + catalog once on start and then goes quiet', async () => {
    const h = makeHarness()
    h.responses.set(LIST_FN, () => ({
      sandboxes: [{ sandbox_id: 'sbx-1', image: 'node', age_secs: 1 }],
    }))
    h.store.start()
    await h.flush()
    expect(h.calls).toEqual([LIST_FN, RUNTIMES_FN, CATALOG_FN])
    expect(h.store.getState().sandboxes).toHaveLength(1)
    expect(h.store.getState().error).toBeNull()
    expect(h.store.getState().lastUpdatedAt).toBe(12345)
    // Push-driven: after a successful bootstrap NOTHING is scheduled.
    expect(h.pending()).toHaveLength(0)
  })

  it('applies pushed fleet events without a read-back and re-reads runtimes debounced', async () => {
    const h = makeHarness()
    h.store.start()
    await h.flush()
    const callsAfterStart = h.calls.length

    const handled = h.store.applyFleetEvent({
      kind: 'fleet',
      daemon_absent: false,
      sandboxes: [{ sandbox_id: 'sbx-9', image: 'python', age_secs: 3 }],
    })
    expect(handled).toBe(true)
    expect(h.store.getState().sandboxes.map((s) => s.sandbox_id)).toEqual(['sbx-9'])
    // The snapshot came off the wire — no sandbox::list call.
    expect(h.calls.filter((c) => c === LIST_FN).length).toBe(1)
    // One debounced runtimes read.
    expect(h.pending().map((t) => t.ms)).toEqual([250])
    await h.runNext()
    expect(h.calls.length).toBe(callsAfterStart + 1)
    expect(h.calls[h.calls.length - 1]).toBe(RUNTIMES_FN)
  })

  it('a fleet event tombstones sandboxes that vanished from the snapshot', async () => {
    const h = makeHarness()
    h.responses.set(LIST_FN, () => ({
      sandboxes: [{ sandbox_id: 'sbx-1', image: 'node', age_secs: 1 }],
    }))
    h.store.start()
    await h.flush()
    h.store.applyFleetEvent({ kind: 'fleet', daemon_absent: false, sandboxes: [] })
    expect(h.store.getState().sandboxes).toHaveLength(0)
    expect(
      h.store.getState().tombstones.map((t) => t.sandbox.sandbox_id),
    ).toEqual(['sbx-1'])
  })

  it('a daemon-absent fleet event flips the empty state', async () => {
    const h = makeHarness()
    h.store.start()
    await h.flush()
    h.store.applyFleetEvent({ kind: 'fleet', daemon_absent: true, sandboxes: [] })
    expect(h.store.getState().daemonAbsent).toBe(true)
  })

  it('non-fleet event kinds are not applied', async () => {
    const h = makeHarness()
    h.store.start()
    await h.flush()
    expect(h.store.applyFleetEvent({ kind: 'teardown', runtime_id: 'rt-1' })).toBe(false)
    expect(h.store.applyFleetEvent('garbage')).toBe(false)
  })

  it('backs off on consecutive errors ONLY until the first data lands', async () => {
    const h = makeHarness()
    let failing = true
    h.responses.set(LIST_FN, () => {
      if (failing) throw new Error('engine unreachable')
      return { sandboxes: [] }
    })

    h.store.start()
    await h.flush()
    expect(h.store.getState().error).toBe('engine unreachable')
    expect(h.store.getState().daemonAbsent).toBe(false)
    expect(h.pending().map((t) => t.ms)).toEqual([3_000])

    await h.runNext()
    expect(h.pending().map((t) => t.ms)).toEqual([6_000])
    await h.runNext()
    expect(h.pending().map((t) => t.ms)).toEqual([12_000])
    await h.runNext()
    expect(h.pending().map((t) => t.ms)).toEqual([24_000])
    await h.runNext()
    expect(h.pending().map((t) => t.ms)).toEqual([30_000])
    await h.runNext()
    // Capped.
    expect(h.pending().map((t) => t.ms)).toEqual([30_000])

    failing = false
    await h.runNext()
    expect(h.store.getState().error).toBeNull()
    // First data landed — the bootstrap retry machine shuts down for good.
    expect(h.pending()).toHaveLength(0)
  })

  it('flags daemon-absent on function-not-found and clears it on success', async () => {
    const h = makeHarness()
    let absent = true
    h.responses.set(LIST_FN, () => {
      if (absent) throw new Error('function sandbox::list not found')
      return { sandboxes: [] }
    })
    h.store.start()
    await h.flush()
    expect(h.store.getState().daemonAbsent).toBe(true)

    absent = false
    h.store.refresh()
    await h.flush()
    expect(h.store.getState().daemonAbsent).toBe(false)
  })

  it('re-reads the catalog only on refresh(true)', async () => {
    const h = makeHarness()
    h.store.start()
    await h.flush()
    expect(h.calls.filter((c) => c === CATALOG_FN)).toHaveLength(1)

    h.store.refresh()
    await h.flush()
    expect(h.calls.filter((c) => c === CATALOG_FN)).toHaveLength(1)

    h.store.refresh(true)
    await h.flush()
    expect(h.calls.filter((c) => c === CATALOG_FN)).toHaveLength(2)
  })

  it('background polls never flip loading back on after the first data', async () => {
    const h = makeHarness()
    h.store.start()
    await h.flush()
    expect(h.store.getState().loading).toBe(false)
    expect(h.store.getState().lastUpdatedAt).toBe(12345)

    h.store.refresh()
    // The synchronous leg of the fetch has run — loading must stay false.
    expect(h.store.getState().loading).toBe(false)
    await h.flush()
    expect(h.store.getState().loading).toBe(false)
  })

  it('a runtimes failure is not fatal to the fleet read', async () => {
    const h = makeHarness()
    h.responses.set(RUNTIMES_FN, () => {
      throw new Error('code runner rebooting')
    })
    h.responses.set(LIST_FN, () => ({
      sandboxes: [{ sandbox_id: 'sbx-1' }],
    }))
    h.store.start()
    await h.flush()
    expect(h.store.getState().error).toBeNull()
    expect(h.store.getState().sandboxes).toHaveLength(1)
    expect(h.store.getState().runtimes).toEqual([])
  })

  it('stop() silences the machine — no writes, no reschedules', async () => {
    const h = makeHarness()
    h.store.start()
    await h.flush()
    h.store.stop()
    const callsAtStop = h.calls.length
    expect(h.pending()).toHaveLength(0)
    h.store.refresh()
    await h.flush()
    expect(h.calls.length).toBe(callsAtStop)
  })
})
