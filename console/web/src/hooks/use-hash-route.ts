import { useCallback, useEffect, useRef, useState } from 'react'

// `chat` is no longer a routed view; it's always-rendered as the side dock
// in App.tsx. Hash routes only pick which view fills the right pane. The
// component spec sheet + streaming playground moved to Storybook, so the
// routed views are `traces`, `workers`, `browser`, and `configuration`.
// `ext` is the injectable-UI prefix: worker-contributed pages route at
// `#/ext/<page-id>` — deliberately outside the first-party names so an
// injected page can never collide with or shadow `#/traces`, `#/workers`, ….
export type View =
  | 'configuration'
  | 'traces'
  | 'workers'
  | 'browser'
  | 'github'
  | 'ext'

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
  if (hash === '#/github') {
    return 'github'
  }
  if (hash === '#/browser' || hash.startsWith('#/browser/')) {
    return 'browser'
  }
  if (hash.startsWith('#/ext/')) {
    return 'ext'
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
    case 'browser':
      return '#/browser'
    case 'github':
      return '#/github'
    case 'configuration':
      return '#/configuration'
    // `ext` needs a page id; navigation to a specific extension page goes
    // through `hashForExtPage`. A bare `#/ext/` resolves to no page and the
    // Ext view falls back to the default view.
    case 'ext':
      return '#/ext/'
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

const EXT_HASH_PREFIX = '#/ext/'

/** `#/ext/<page-id>` -> the page id, or null for anything else. */
export function extPageFromHash(hash: string): string | null {
  if (!hash.startsWith(EXT_HASH_PREFIX)) return null
  const segment = hash
    .slice(EXT_HASH_PREFIX.length)
    .split('/')
    .filter(Boolean)[0]
  if (!segment) return null
  return decodeSegment(segment)
}

export function hashForExtPage(pageId: string): string {
  return `${EXT_HASH_PREFIX}${encodeURIComponent(pageId)}`
}

/**
 * Selected extension page as a hash sub-route, so injected pages deep-link
 * (`#/ext/<page-id>`) and survive reloads.
 */
export function useExtPageRoute(): string | null {
  const [selected, setSelected] = useState<string | null>(() => {
    if (typeof window === 'undefined') return null
    return extPageFromHash(window.location.hash)
  })
  const selectedRef = useRef(selected)
  selectedRef.current = selected

  useEffect(() => {
    const sync = () => {
      const next = extPageFromHash(window.location.hash)
      if (next !== selectedRef.current) setSelected(next)
    }
    sync()
    window.addEventListener('hashchange', sync)
    return () => window.removeEventListener('hashchange', sync)
  }, [])

  return selected
}

const BROWSER_HASH = '#/browser'

/** `#/browser/<session_id>` -> the session id, or null for `#/browser`. */
export function browserSessionFromHash(hash: string): string | null {
  if (!hash.startsWith(`${BROWSER_HASH}/`)) return null
  const segment = hash
    .slice(BROWSER_HASH.length + 1)
    .split('/')
    .filter(Boolean)[0]
  if (!segment) return null
  return decodeSegment(segment)
}

export function hashForBrowserSession(sessionId: string | null): string {
  if (!sessionId) return BROWSER_HASH
  return `${BROWSER_HASH}/${encodeURIComponent(sessionId)}`
}

/**
 * Selected browser session as a hash sub-route, so sessions deep-link
 * (`#/browser/<session_id>`) from chat cards and survive reloads.
 */
export function useBrowserSessionRoute(): [
  string | null,
  (next: string | null) => void,
] {
  const [selected, setSelected] = useState<string | null>(() => {
    if (typeof window === 'undefined') return null
    return browserSessionFromHash(window.location.hash)
  })
  const selectedRef = useRef(selected)
  selectedRef.current = selected

  useEffect(() => {
    const sync = () => {
      const next = browserSessionFromHash(window.location.hash)
      if (next !== selectedRef.current) setSelected(next)
    }
    sync()
    window.addEventListener('hashchange', sync)
    return () => window.removeEventListener('hashchange', sync)
  }, [])

  const navigate = useCallback((next: string | null) => {
    const targetHash = hashForBrowserSession(next)
    if (window.location.hash !== targetHash) {
      window.location.hash = targetHash
    } else {
      setSelected(next)
    }
  }, [])

  return [selected, navigate]
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
