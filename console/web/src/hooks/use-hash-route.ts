import { useCallback, useEffect, useRef, useState } from 'react'

// `chat` is no longer a routed view; it's always-rendered as the side dock
// in App.tsx. Hash routes only pick which view fills the right pane. The
// component spec sheet + streaming playground moved to Storybook, so the
// only routed views left are `traces` and `configuration`.
export type View = 'configuration' | 'traces' | 'workers' | 'timeline' | 'triggers'

/**
 * Sub-tab inside the Configuration page. URL-driven so deep links and the
 * back button move between settings surfaces cleanly. `console` covers the
 * existing console-level preferences (theme + provider api keys); `workers`
 * is the worker-config registry surfaced through `configuration::list`.
 */
export type ConfigurationTab = 'console' | 'workers'

export interface ConfigurationRoute {
  tab: ConfigurationTab
  /** Selected worker id when `tab === 'workers'`. */
  workerId: string | null
}

function routeFromHash(hash: string): View | null {
  if (hash === '' || hash === '#' || hash === '#/' || hash === '#/traces') {
    return 'traces'
  }
  // Backwards compat: `#/chat` no longer exists as a view -- chat is the
  // always-visible side dock now. Land legacy bookmarks on the default view.
  if (hash === '#/chat') return 'traces'
  if (hash === '#/workers') {
    return 'workers'
  }
  if (hash === '#/configuration' || hash.startsWith('#/configuration/')) {
    return 'configuration'
  }
  // Backwards compat: `#/providers` was renamed to `#/configuration` when
  // the page absorbed the theme toggle and any other future settings.
  if (hash === '#/providers') return 'configuration'
  // Legacy dev-only routes: the spec sheet and streaming playground moved to
  // Storybook, so old bookmarks land on the default view instead of 404ing.
  if (hash === '#/examples' || hash === '#/playground') return 'traces'
  if (hash === '#/triggers') return 'triggers'
  if (hash === '#/timeline' || hash.startsWith('#/timeline/')) return 'timeline'
  return null
}

function hashFor(view: View): string {
  switch (view) {
    case 'traces':
      return '#/traces'
    case 'workers':
      return '#/workers'
    case 'configuration':
      return '#/configuration'
    case 'timeline':
      return '#/timeline'
    case 'triggers':
      return '#/triggers'
  }
}

/**
 * Parse the configuration sub-route out of a hash. Returns sensible defaults
 * for legacy or unrecognized values so a typo in the URL bar never strands
 * the operator on an empty page.
 */
function configRouteFromHash(hash: string): ConfigurationRoute {
  // Treat bare `#/configuration` and any non-configuration hash as the
  // default `console` tab. Legacy `#/providers` is funneled here too.
  if (!hash.startsWith('#/configuration/')) {
    return { tab: 'console', workerId: null }
  }
  const rest = hash.slice('#/configuration/'.length)
  const segments = rest.split('/').filter(Boolean)
  const [first, second] = segments
  if (first === 'workers') {
    return {
      tab: 'workers',
      workerId: second ? safeDecode(second) : null,
    }
  }
  return { tab: 'console', workerId: null }
}

function hashForConfigRoute(route: ConfigurationRoute): string {
  if (route.tab === 'workers') {
    return route.workerId
      ? `#/configuration/workers/${encodeURIComponent(route.workerId)}`
      : '#/configuration/workers'
  }
  return '#/configuration/console'
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

/**
 * Sub-routing for the Configuration page. Mirrors `useHashRoute` so the
 * two stay in lockstep when the operator navigates by hash, by tab click,
 * or by selecting a worker from the list.
 *
 * Navigation goes through `window.location.hash` so the browser back/forward
 * buttons traverse the configuration history; falling back to local state
 * when the hash is already at the target lets sibling components react to
 * navigation calls that are no-ops at the URL level (rare, but defensive).
 */
export function useConfigurationRoute(): [
  ConfigurationRoute,
  (next: Partial<ConfigurationRoute>) => void,
] {
  const [route, setRoute] = useState<ConfigurationRoute>(() => {
    if (typeof window === 'undefined') return { tab: 'console', workerId: null }
    return configRouteFromHash(window.location.hash)
  })
  const routeRef = useRef(route)
  routeRef.current = route

  useEffect(() => {
    const handle = () => {
      const next = configRouteFromHash(window.location.hash)
      const cur = routeRef.current
      if (next.tab !== cur.tab || next.workerId !== cur.workerId) {
        setRoute(next)
      }
    }
    window.addEventListener('hashchange', handle)
    return () => window.removeEventListener('hashchange', handle)
  }, [])

  const navigate = useCallback((next: Partial<ConfigurationRoute>) => {
    const cur = routeRef.current
    const merged: ConfigurationRoute = {
      tab: next.tab ?? cur.tab,
      // Switching to console clears the worker selection so back-navigating
      // doesn't reopen a previously-edited worker out of context.
      workerId:
        next.tab && next.tab !== 'workers'
          ? null
          : next.workerId !== undefined
            ? next.workerId
            : cur.workerId,
    }
    const targetHash = hashForConfigRoute(merged)
    if (window.location.hash !== targetHash) {
      window.location.hash = targetHash
    } else {
      setRoute(merged)
    }
  }, [])

  return [route, navigate]
}

function timelineSessionFromHash(hash: string): string | null {
  if (!hash.startsWith('#/timeline/')) return null
  const rest = hash.slice('#/timeline/'.length)
  return rest ? safeDecode(rest) : null
}

/** decodeURIComponent that survives malformed percent-encoding in shared
 * links — a URIError thrown from a useState initializer would blank the
 * whole SPA (no ErrorBoundary above the routed views). */
function safeDecode(segment: string): string | null {
  try {
    return decodeURIComponent(segment)
  } catch {
    return null
  }
}

/** Session-id sub-route for the Timeline page (`#/timeline/<sessionId>`). */
export function useTimelineRoute(): [
  string | null,
  (sessionId: string | null) => void,
] {
  const [sessionId, setSessionId] = useState<string | null>(() => {
    if (typeof window === 'undefined') return null
    return timelineSessionFromHash(window.location.hash)
  })
  const ref = useRef(sessionId)
  ref.current = sessionId

  useEffect(() => {
    const handle = () => {
      const next = timelineSessionFromHash(window.location.hash)
      if (next !== ref.current) setSessionId(next)
    }
    window.addEventListener('hashchange', handle)
    return () => window.removeEventListener('hashchange', handle)
  }, [])

  const navigate = useCallback((next: string | null) => {
    const targetHash = next
      ? `#/timeline/${encodeURIComponent(next)}`
      : '#/timeline'
    if (window.location.hash !== targetHash) {
      window.location.hash = targetHash
    } else {
      setSessionId(next)
    }
  }, [])

  return [sessionId, navigate]
}
