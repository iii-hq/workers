/**
 * React hooks shared by the Console and injected worker UI. Imports `react`,
 * which stays external in worker builds (the import map resolves it to the
 * Console's single React copy). Bundleable: `@iii-dev/console-ui/hooks`.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
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
 * Live worker data: `fetch()` once, re-fetch on every one of `triggers`
 * (a type, or `{ type, config }` for a stream/filtered binding; each bound
 * to one tab-scoped handler `handlerId::<browserId>`). There is no timer:
 * while a binding is missing (`live: false`), the data is re-read when the
 * tab becomes visible or focused again, and on `refresh()`. Stale responses
 * are dropped by a monotonic token; everything unbinds on cleanup.
 * `pollMs` is accepted for compatibility and ignored.
 */
export function useWorkerLive({ iii, triggers, fetch, handlerId }) {
  const [data, setData] = useState(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)
  const [live, setLive] = useState(false)
  const [token, setToken] = useState(0)
  const seq = useRef(0)

  const fetchRef = useRef(fetch)
  fetchRef.current = fetch
  const refresh = useCallback(() => setToken((t) => t + 1), [])

  // Keyed on the trigger list's contents so an inline array literal does
  // not rebind every render.
  const triggerKey = triggers.map((t) => (typeof t === 'string' ? t : JSON.stringify(t))).join('\u0000')
  useEffect(() => {
    const offs = []
    let ok = triggers.length > 0
    try {
      offs.push(iii.on(handlerId, refresh))
      for (const t of triggers) {
        const { type, config = {} } = typeof t === 'string' ? { type: t } : t
        offs.push(
          iii.registerTrigger({
            type,
            function_id: `${handlerId}::${iii.browserId}`,
            config,
          }),
        )
      }
    } catch {
      // Worker absent or trigger type unregistered — re-read on tab focus.
      ok = false
    }
    setLive(ok)
    return () => {
      setLive(false)
      for (const off of offs) {
        try {
          off()
        } catch {
          // already gone
        }
      }
    }
    // biome-ignore lint/correctness/useExhaustiveDependencies: triggerKey stands in for the array's contents
  }, [iii, handlerId, triggerKey, refresh])

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

  // Without a live binding, catch up when the person comes back to the tab
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
