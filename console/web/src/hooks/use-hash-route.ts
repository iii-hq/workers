import { useCallback, useEffect, useRef, useState } from 'react'

// `chat` is no longer a routed view; it's always-rendered as the side dock
// in App.tsx. Hash routes only pick which view fills the right pane. The
// component spec sheet + streaming playground moved to Storybook, so the
// routed views are `traces`, `workers`, `worktrees`, and `configuration`.
export type View = 'configuration' | 'traces' | 'workers' | 'worktrees'

export interface WorkersConfigurationRoute {
  configurationId: string | null
  fieldPath: string[]
}

const WORKERS_CONFIGURATION_PREFIX = '#/workers/configuration/'
const LEGACY_WORKERS_CONFIGURATION_PREFIX = '#/configuration/workers/'

function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment)
  } catch {
    return segment
  }
}

function encodePath(segments: string[]): string {
  return segments.map((segment) => encodeURIComponent(segment)).join('/')
}

function parseConfigurationRouteWithPrefix(
  hash: string,
  prefix: string,
): WorkersConfigurationRoute | null {
  if (!hash.startsWith(prefix)) return null
  const segments = hash
    .slice(prefix.length)
    .split('/')
    .filter(Boolean)
    .map(decodeSegment)
  const [configurationId, ...fieldPath] = segments
  if (!configurationId) return { configurationId: null, fieldPath: [] }
  return { configurationId, fieldPath }
}

export function workersConfigurationRouteFromHash(
  hash: string,
): WorkersConfigurationRoute {
  return (
    parseConfigurationRouteWithPrefix(hash, WORKERS_CONFIGURATION_PREFIX) ??
    parseConfigurationRouteWithPrefix(
      hash,
      LEGACY_WORKERS_CONFIGURATION_PREFIX,
    ) ?? { configurationId: null, fieldPath: [] }
  )
}

export function hashForWorkersConfiguration(
  configurationId: string,
  fieldPath: string[] = [],
): string {
  const suffix = encodePath([configurationId, ...fieldPath])
  return `${WORKERS_CONFIGURATION_PREFIX}${suffix}`
}

export function normalizeWorkersConfigurationHash(hash: string): string | null {
  if (hash === '#/configuration/workers') return '#/workers'
  const legacy = parseConfigurationRouteWithPrefix(
    hash,
    LEGACY_WORKERS_CONFIGURATION_PREFIX,
  )
  if (!legacy?.configurationId) return null
  return hashForWorkersConfiguration(legacy.configurationId, legacy.fieldPath)
}

function routeFromHash(hash: string): View | null {
  if (hash === '' || hash === '#' || hash === '#/' || hash === '#/traces') {
    return 'traces'
  }
  // Backwards compat: `#/chat` no longer exists as a view -- chat is the
  // always-visible side dock now. Land legacy bookmarks on the default view.
  if (hash === '#/chat') return 'traces'
  // Backwards compat: `#/traces-v2` was the staging route while the rebuilt
  // traces view coexisted with the original; it IS `#/traces` now.
  if (hash === '#/traces-v2') return 'traces'
  if (hash === '#/workers' || hash.startsWith('#/workers/')) {
    return 'workers'
  }
  if (hash === '#/worktrees') {
    return 'worktrees'
  }
  if (hash === '#/configuration/workers') {
    return 'workers'
  }
  if (hash.startsWith(LEGACY_WORKERS_CONFIGURATION_PREFIX)) {
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
  return null
}

function hashFor(view: View): string {
  switch (view) {
    case 'traces':
      return '#/traces'
    case 'workers':
      return '#/workers'
    case 'worktrees':
      return '#/worktrees'
    case 'configuration':
      return '#/configuration'
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

function replaceHash(targetHash: string) {
  window.history.replaceState(
    window.history.state,
    '',
    `${window.location.pathname}${window.location.search}${targetHash}`,
  )
}

export function useWorkersConfigurationRoute(): [
  WorkersConfigurationRoute,
  (configurationId: string | null, fieldPath?: string[]) => void,
] {
  const [route, setRoute] = useState<WorkersConfigurationRoute>(() => {
    if (typeof window === 'undefined') {
      return { configurationId: null, fieldPath: [] }
    }
    return workersConfigurationRouteFromHash(window.location.hash)
  })
  const routeRef = useRef(route)
  routeRef.current = route

  useEffect(() => {
    const sync = () => {
      const normalized = normalizeWorkersConfigurationHash(window.location.hash)
      if (normalized && normalized !== window.location.hash) {
        replaceHash(normalized)
      }
      const next = workersConfigurationRouteFromHash(window.location.hash)
      const cur = routeRef.current
      if (
        next.configurationId !== cur.configurationId ||
        next.fieldPath.join('/') !== cur.fieldPath.join('/')
      ) {
        setRoute(next)
      }
    }
    sync()
    window.addEventListener('hashchange', sync)
    return () => window.removeEventListener('hashchange', sync)
  }, [])

  const navigate = useCallback(
    (configurationId: string | null, fieldPath: string[] = []) => {
      const targetHash = configurationId
        ? hashForWorkersConfiguration(configurationId, fieldPath)
        : '#/workers'
      if (window.location.hash !== targetHash) {
        window.location.hash = targetHash
      } else {
        setRoute(workersConfigurationRouteFromHash(targetHash))
      }
    },
    [],
  )

  return [route, navigate]
}
