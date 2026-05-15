import { useCallback, useEffect, useRef, useState } from 'react'

export type View = 'chat' | 'examples' | 'playground'

const PLAYGROUND_ENABLED = !!import.meta.env.VITE_PLAYGROUND

/**
 * Resolve a hash to a route view, OR null if it isn't a route hash at all
 * (e.g. anchor links like `#message-variants`). When it's not a route, the
 * caller should keep the current view rather than reset to chat.
 *
 * When the playground flag is off, dev-only routes resolve to 'chat' so old
 * deep links don't strand a user on a 404.
 */
function routeFromHash(hash: string): View | null {
  if (hash === '' || hash === '#' || hash === '#/' || hash === '#/chat') {
    return 'chat'
  }
  if (hash === '#/examples') return PLAYGROUND_ENABLED ? 'examples' : 'chat'
  if (hash === '#/playground') return PLAYGROUND_ENABLED ? 'playground' : 'chat'
  return null
}

export function useHashRoute(): [View, (next: View) => void] {
  /* Initial: read window.location on mount. */
  const [view, setView] = useState<View>(() => {
    if (typeof window === 'undefined') return 'chat'
    return routeFromHash(window.location.hash) ?? 'chat'
  })
  const viewRef = useRef(view)
  viewRef.current = view

  useEffect(() => {
    const handle = () => {
      const next = routeFromHash(window.location.hash)
      /* Only react to actual route hashes; anchor hashes keep the current view. */
      if (next !== null && next !== viewRef.current) setView(next)
    }
    window.addEventListener('hashchange', handle)
    return () => window.removeEventListener('hashchange', handle)
  }, [])

  const navigate = useCallback((next: View) => {
    const targetHash =
      next === 'chat'
        ? '#/'
        : next === 'examples'
          ? '#/examples'
          : '#/playground'
    if (window.location.hash !== targetHash) {
      window.location.hash = targetHash
    } else {
      setView(next)
    }
  }, [])

  return [view, navigate]
}
