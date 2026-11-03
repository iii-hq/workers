/**
 * React hooks shared by the Console and injected worker UI. Imports `react`,
 * which stays external in worker builds (the import map resolves it to the
 * Console's single React copy). Bundleable: `@iii-dev/console-ui/hooks`.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { copyText, errorMessage } from './format.mjs'

/**
 * Container-driven "is this pane narrow?" — a ResizeObserver on the node,
 * not a viewport media query, so the same component adapts inside any pane
 * the Console gives it. Measures synchronously when the ref attaches (no
 * wide-mode flash); zero widths (display:none hosts) keep the last layout.
 */
export function useContainerNarrow({ below = 720 } = {}) {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef(null)
  const ref = useCallback(
    (node) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < below)
      if (typeof ResizeObserver === 'undefined') return
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < below)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [below],
  )
  return { ref, narrow }
}

/** `value`, settled: updates only after `ms` without a change (remote search queries). */
export function useDebounce(value, ms = 200) {
  const [settled, setSettled] = useState(value)
  useEffect(() => {
    const id = setTimeout(() => setSettled(value), ms)
    return () => clearTimeout(id)
  }, [value, ms])
  return settled
}

/**
 * One drag/keyboard resizer for a split separator. The caller keeps
 * `role="separator"` and its `aria-value*` on the element and spreads the
 * returned handlers; pointer capture makes the drag survive leaving the
 * handle. Arrow keys step along the axis: Left/Up is -1, Right/Down is +1.
 */
export function useSplitDrag({ horizontal, begin, move, step }) {
  const dragRef = useRef(null)
  const coordinate = (event) => (horizontal ? event.clientX : event.clientY)
  const onPointerDown = (event) => {
    const origin = begin(event)
    if (origin === null) return
    dragRef.current = { start: coordinate(event), origin }
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
  }
  const onPointerMove = (event) => {
    const drag = dragRef.current
    if (drag) move(drag.origin, coordinate(event) - drag.start)
  }
  const onPointerUp = (event) => {
    dragRef.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }
  const onKeyDown = (event) => {
    const [back, forward] = horizontal ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown']
    if (event.key !== back && event.key !== forward) return
    event.preventDefault()
    step(event.key === back ? -1 : 1, event)
  }
  return {
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel: onPointerUp,
    onLostPointerCapture: onPointerUp,
    onKeyDown,
  }
}

/** `useState` mirrored to localStorage as JSON, best effort (private mode, quota). */
export function usePaneState(key, initial) {
  const [value, setValue] = useState(() => {
    try {
      const raw = window.localStorage.getItem(key)
      return raw === null ? initial : JSON.parse(raw)
    } catch {
      return initial
    }
  })
  const update = useCallback(
    (next) => {
      setValue((prev) => {
        const resolved = typeof next === 'function' ? next(prev) : next
        try {
          window.localStorage.setItem(key, JSON.stringify(resolved))
        } catch {
          // best effort
        }
        return resolved
      })
    },
    [key],
  )
  return [value, update]
}

/**
 * Click-to-copy with a `copied`/`failed` flash. A rapid second click
 * extends the flash instead of letting the first timer cut it short; the
 * timer is cleared on unmount.
 */
export function useCopyFlash(text, ms = 1400) {
  const [state, setState] = useState('idle')
  const timer = useRef(null)
  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current)
    },
    [],
  )
  const copy = useCallback(() => {
    void copyText(text).then((ok) => {
      setState(ok ? 'copied' : 'failed')
      if (timer.current !== null) clearTimeout(timer.current)
      timer.current = setTimeout(() => {
        timer.current = null
        setState('idle')
      }, ms)
    })
  }, [text, ms])
  return { state, copy }
}

/**
 * The 0.25 trigger types whose 0.26 source function the naming rule does not
 * give (workers contract `tech-specs/2026-10-observe-v2/functions.md` §1.1;
 * the console's `RENAMED_SOURCES`). `subscribe` is left out: its topic lived
 * in the binding's config, which has no reliable mapping.
 */
const RENAMED_TRIGGER_TYPES = {
  cron: 'cron::on-tick',
  http: 'http::on-request',
  'durable:subscriber': 'queue::on-message',
  'database::row-changed': 'database::on-row-changed',
  state: 'state::on-changed',
  'compose-operation': 'compose::on-operation-progress',
  'console:script': 'ade::ui::asset',
  'console:style': 'ade::ui::asset',
  'console:module': 'ade::ui::asset',
  'console:assets': 'ade::ui::on-assets-changed',
  timer: 'harness::on-timer',
  'harness::triggers-changed': 'harness::on-observers-changed',
  'harness::hook::pre-trigger': 'harness::hook::on-pre-call',
  'harness::hook::post-trigger': 'harness::hook::on-post-call',
  log: 'observability::on-log',
  trace: 'observability::on-span',
  configuration: 'configuration::on-updated',
  'engine::functions-available': 'engine::on-functions-changed',
  'engine::workers-available': 'engine::on-workers-changed',
  stream: 'stream::on-changed',
  'stream:join': 'stream::on-join',
  'stream:leave': 'stream::on-leave',
}

/** The source function of a 0.25 trigger type: the table, then the naming rule (`W::X` → `W::on-X`, `a:b` → `a::on-b`). */
function sourceOfTriggerType(type) {
  const renamed = RENAMED_TRIGGER_TYPES[type]
  if (renamed) return renamed
  if (type.includes('::')) {
    const at = type.lastIndexOf('::')
    const last = type.slice(at + 2)
    return last.startsWith('on-') ? type : `${type.slice(0, at)}::on-${last}`
  }
  const colon = type.indexOf(':')
  return colon > 0 ? `${type.slice(0, colon)}::on-${type.slice(colon + 1)}` : null
}

/**
 * The 0.26 sources of a deprecated 0.25 `triggers` list. Only a bare type
 * maps reliably; a binding `config` (filters, args, a topic) does not, so
 * such entries are skipped with a warning and the page re-reads on focus.
 */
function sourcesOfTriggers(triggers) {
  const sources = []
  for (const t of triggers) {
    const { type, config } = typeof t === 'string' ? { type: t } : (t ?? {})
    const hasConfig = config && typeof config === 'object' && Object.keys(config).length > 0
    const source = typeof type === 'string' && type !== 'subscribe' && !hasConfig ? sourceOfTriggerType(type) : null
    if (source) sources.push(source)
    else console.warn(`[console-ui] useWorkerLive: trigger ${JSON.stringify(t)} has no 0.26 source; pass \`sources\` instead`)
  }
  if (sources.length > 0) {
    console.warn(`[console-ui] useWorkerLive: \`triggers\` is deprecated; observing sources ${sources.join(', ')}`)
  }
  return sources
}

const keyOf = (list) => list.map((s) => (typeof s === 'string' ? s : JSON.stringify(s))).join('\u0000')

/** How long the catch-up re-fetch waits for the observers to turn active. */
const CATCH_UP_READY_WAIT_MS = 3000

/**
 * Run `read()` once every stateful observer is active (or after a bounded
 * wait, so an absent worker never stalls it), then again whenever one turns
 * `active` from any other state (a reconnect replays the observer through
 * `suspended`/`pending`; the calls in between are lost). An observer only
 * receives calls made once it is active, so a read before that can miss an
 * event. Observers without `ready`/`onStateChange` (an older console) are
 * left alone. Returns the stop function.
 */
function catchUpWhenActive(observers, read, timeoutMs = CATCH_UP_READY_WAIT_MS) {
  const tracked = observers.filter((off) => typeof off?.ready === 'function' && typeof off?.onStateChange === 'function')
  if (tracked.length === 0) return () => {}
  let stopped = false
  let caughtUp = false
  const unwatch = []
  for (const off of tracked) {
    let previous = off.state
    try {
      unwatch.push(
        off.onStateChange((state) => {
          const was = previous
          previous = state
          if (state === 'active' && was !== 'active' && caughtUp && !stopped) read()
        }),
      )
    } catch {
      // An observer that cannot report its state: the first read stands.
    }
  }
  let timer
  const waits = tracked.map((off) => {
    try {
      return Promise.resolve(off.ready()).catch(() => undefined)
    } catch {
      return Promise.resolve()
    }
  })
  void Promise.race([Promise.all(waits), new Promise((resolve) => (timer = setTimeout(resolve, timeoutMs)))]).then(() => {
    clearTimeout(timer)
    if (stopped) return
    caughtUp = true
    read()
  })
  return () => {
    stopped = true
    clearTimeout(timer)
    for (const off of unwatch.splice(0)) {
      try {
        off()
      } catch {
        // already gone
      }
    }
  }
}

/**
 * Live worker data: `fetch()` once, re-fetch on every call of the observed
 * `sources` (a function id, or `{ function_id, args?, filters? }` for one
 * that takes args or a filtered feed; each observed with the one tab-scoped
 * handler `handlerId::<browserId>` as target), once more when every observer
 * has turned active (events fired while one was pending are lost) and again
 * whenever one comes back active after a reconnect. There is no timer: while the
 * observers are not all active (`live: false` — pending, refused, or
 * disconnected), the data is re-read when the tab becomes visible or focused
 * again, and on `refresh()`. Stale responses are dropped by a monotonic
 * token; every observer is disposed on cleanup. `pollMs` is accepted for
 * compatibility and ignored; the 0.25 `triggers` is a deprecated alias for
 * bare trigger types, mapped to their source functions.
 */
export function useWorkerLive({ iii, sources, triggers, fetch, handlerId }) {
  const [data, setData] = useState(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)
  const [live, setLive] = useState(false)
  const [token, setToken] = useState(0)
  const seq = useRef(0)

  const fetchRef = useRef(fetch)
  fetchRef.current = fetch
  const refresh = useCallback(() => setToken((t) => t + 1), [])

  const legacyKey = !sources && Array.isArray(triggers) ? keyOf(triggers) : ''
  // biome-ignore lint/correctness/useExhaustiveDependencies: legacyKey stands in for the array's contents
  const legacy = useMemo(() => (legacyKey ? sourcesOfTriggers(triggers) : []), [legacyKey])
  const observed = sources ?? legacy
  // Keyed on the source list's contents so an inline array literal does
  // not observe again every render.
  const sourceKey = keyOf(observed)
  useEffect(() => {
    const offs = []
    const observers = []
    const unwatch = []
    let failed = observed.length === 0
    try {
      offs.push(iii.on(handlerId, refresh))
      const target = `${handlerId}::${iii.browserId}`
      for (const source of observed) {
        const off = iii.observe(source, target)
        offs.push(off)
        observers.push(off)
      }
    } catch {
      // The client could not observe — re-read on tab focus.
      failed = true
    }
    // Live only while every observer is active: a refused or still-pending
    // one keeps the focus re-read on. A console whose disposer carries no
    // state (older) counts an observer live once `observe` returned.
    const tracked = (off) => typeof off?.onStateChange === 'function'
    const update = () => setLive(!failed && observers.every((off) => !tracked(off) || off.state === 'active'))
    for (const off of observers) if (tracked(off)) unwatch.push(off.onStateChange(update))
    update()
    // The first fetch may run before the observers are active: fetch again
    // once they are, and whenever one comes back after a reconnect.
    if (!failed) unwatch.push(catchUpWhenActive(observers, refresh))
    return () => {
      setLive(false)
      for (const off of [...unwatch, ...offs]) {
        try {
          off()
        } catch {
          // already gone
        }
      }
    }
    // biome-ignore lint/correctness/useExhaustiveDependencies: sourceKey stands in for the array's contents
  }, [iii, handlerId, sourceKey, refresh])

  // biome-ignore lint/correctness/useExhaustiveDependencies: token is a re-run token (events, poll, manual refresh)
  useEffect(() => {
    const mine = ++seq.current
    setLoading(true)
    void Promise.resolve()
      .then(() => fetchRef.current())
      .then(
        (next) => {
          if (mine !== seq.current) return
          setData(next)
          setError(null)
        },
        (err) => {
          if (mine !== seq.current) return
          setError(errorMessage(err))
        },
      )
      .finally(() => {
        if (mine === seq.current) setLoading(false)
      })
  }, [iii, token])

  // Without live observers, catch up when the person comes back to the tab
  // instead of re-reading on a timer.
  useEffect(() => {
    if (live || typeof window === 'undefined') return
    const onVisible = () => {
      if (typeof document !== 'undefined' && document.hidden) return
      refresh()
    }
    window.addEventListener('focus', onVisible)
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      window.removeEventListener('focus', onVisible)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [live, refresh])

  return { data, loading, error, live, refresh }
}
