import { useCallback, useEffect, useRef, useState } from 'react'

// `chat` is no longer a routed view; it's always-rendered as the side dock
// in App.tsx. Hash routes only pick which view fills the right pane.
export type View = 'configuration' | 'examples' | 'playground' | 'traces'

const PLAYGROUND_ENABLED = !!import.meta.env.VITE_PLAYGROUND

function routeFromHash(hash: string): View | null {
  if (hash === '' || hash === '#' || hash === '#/' || hash === '#/traces') {
    return 'traces'
  }
  // Backwards compat: `#/chat` no longer exists as a view -- chat is the
  // always-visible side dock now. Land legacy bookmarks on the default view.
  if (hash === '#/chat') return 'traces'
  if (hash === '#/configuration') return 'configuration'
  // Backwards compat: `#/providers` was renamed to `#/configuration` when
  // the page absorbed the theme toggle and any other future settings.
  if (hash === '#/providers') return 'configuration'
  if (hash === '#/examples') return PLAYGROUND_ENABLED ? 'examples' : 'traces'
  if (hash === '#/playground') return PLAYGROUND_ENABLED ? 'playground' : 'traces'
  return null
}

function hashFor(view: View): string {
  switch (view) {
    case 'traces':
      return '#/traces'
    case 'configuration':
      return '#/configuration'
    case 'examples':
      return '#/examples'
    case 'playground':
      return '#/playground'
  }
}

export function useHashRoute(): [View, (next: View) => void] {
  const [view, setView] = useState<View>(() => {
    if (typeof window === 'undefined') return 'traces'
    return routeFromHash(window.location.hash) ?? 'traces'
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
    const targetHash = hashFor(next)
    if (window.location.hash !== targetHash) {
      window.location.hash = targetHash
    } else {
      setView(next)
    }
  }, [])

  return [view, navigate]
}
