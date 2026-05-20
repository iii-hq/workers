import { useCallback, useEffect, useRef, useState } from 'react'

export type View = 'chat' | 'examples' | 'playground' | 'traces'

const PLAYGROUND_ENABLED = !!import.meta.env.VITE_PLAYGROUND

function routeFromHash(hash: string): View | null {
  if (hash === '' || hash === '#' || hash === '#/' || hash === '#/chat') {
    return 'chat'
  }
  if (hash === '#/traces') return 'traces'
  if (hash === '#/examples') return PLAYGROUND_ENABLED ? 'examples' : 'chat'
  if (hash === '#/playground') return PLAYGROUND_ENABLED ? 'playground' : 'chat'
  return null
}

export function useHashRoute(): [View, (next: View) => void] {
  const [view, setView] = useState<View>(() => {
    if (typeof window === 'undefined') return 'chat'
    return routeFromHash(window.location.hash) ?? 'chat'
  })
  const viewRef = useRef(view)
  viewRef.current = view

  useEffect(() => {
    const handle = () => {
      const next = routeFromHash(window.location.hash)
      if (next !== null && next !== viewRef.current) setView(next)
    }
    window.addEventListener('hashchange', handle)
    return () => window.removeEventListener('hashchange', handle)
  }, [])

  const navigate = useCallback((next: View) => {
    const targetHash =
      next === 'chat'
        ? '#/'
        : next === 'traces'
          ? '#/traces'
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
