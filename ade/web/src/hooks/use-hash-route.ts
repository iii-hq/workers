import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import type { JsonValue } from '@/types/injectable-ui'

// `chat` is no longer a routed view; it's always-rendered as the side dock
// in App.tsx. Hash routes only pick which view fills the right pane. The
// component spec sheet + streaming playground moved to Storybook, so the
// first-party routed views are `traces`, `workers`, and `configuration`.
// Worker pages have no workspace route: they open through `host.panels.open`
// or `console::workspace::open` and live in the tab store. Their one hash is
// `#/worker/<scope>` — the isolated shell (WorkerOnly.tsx) that renders a
// single injected page with no workspace at all.
export type View = 'configuration' | 'traces' | 'workers'

export interface WorkersConfigurationRoute {
  /**
   * Whether the configuration surface is showing at all. Distinct from
   * `configurationId` so the narrow drill-in flow can sit on the list
   * (`#/configuration/workers`, open with nothing selected) without
   * falling back to General settings.
   */
  open: boolean
  configurationId: string | null
  fieldPath: string[]
}

const CLOSED_CONFIGURATION_ROUTE: WorkersConfigurationRoute = {
  open: false,
  configurationId: null,
  fieldPath: [],
}

const WORKERS_CONFIGURATION_ROOT = '#/configuration/workers'
const WORKERS_CONFIGURATION_PREFIX = '#/configuration/workers/'
const LEGACY_WORKERS_CONFIGURATION_ROOT = '#/workers/configuration'
const LEGACY_WORKERS_CONFIGURATION_PREFIX = '#/workers/configuration/'

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
  if (!configurationId)
    return { open: true, configurationId: null, fieldPath: [] }
  return { open: true, configurationId, fieldPath }
}

export function workersConfigurationRouteFromHash(
  hash: string,
): WorkersConfigurationRoute {
  // Bare `#/configuration/workers`: the surface is open with nothing
  // selected — the narrow flow's list page.
  if (
    hash === WORKERS_CONFIGURATION_ROOT ||
    hash === LEGACY_WORKERS_CONFIGURATION_ROOT
  ) {
    return { open: true, configurationId: null, fieldPath: [] }
  }
  return (
    parseConfigurationRouteWithPrefix(hash, WORKERS_CONFIGURATION_PREFIX) ??
    parseConfigurationRouteWithPrefix(
      hash,
      LEGACY_WORKERS_CONFIGURATION_PREFIX,
    ) ??
    CLOSED_CONFIGURATION_ROUTE
  )
}

export function hashForWorkersConfiguration(
  configurationId: string,
  fieldPath: string[] = [],
): string {
  const suffix = encodePath([configurationId, ...fieldPath])
  return `${WORKERS_CONFIGURATION_PREFIX}${suffix}`
}

export function hashForWorkersConfigurationList(): string {
  return WORKERS_CONFIGURATION_ROOT
}

/** The settings entry route: navigation first on narrow screens, General otherwise. */
export function hashForSettingsLanding(narrow: boolean): string {
  return narrow
    ? hashForWorkersConfigurationList()
    : hashForView('configuration')
}

export function normalizeWorkersConfigurationHash(hash: string): string | null {
  if (
    hash === LEGACY_WORKERS_CONFIGURATION_ROOT ||
    hash === `${LEGACY_WORKERS_CONFIGURATION_ROOT}/`
  ) {
    return WORKERS_CONFIGURATION_ROOT
  }
  const legacy = parseConfigurationRouteWithPrefix(
    hash,
    LEGACY_WORKERS_CONFIGURATION_PREFIX,
  )
  if (!legacy?.configurationId) return null
  return hashForWorkersConfiguration(legacy.configurationId, legacy.fieldPath)
}

export function routeFromHash(hash: string): View | null {
  if (hash === '' || hash === '#' || hash === '#/' || hash === '#/traces') {
    return 'traces'
  }
  // Backwards compat: `#/chat` no longer exists as a view -- chat is the
  // always-visible side dock now. Land legacy bookmarks on the default view.
  if (hash === '#/chat') return 'traces'
  // Backwards compat: `#/traces-v2` was the staging route while the rebuilt
  // traces view coexisted with the original; it IS `#/traces` now.
  if (hash === '#/traces-v2') return 'traces'
  if (
    hash === LEGACY_WORKERS_CONFIGURATION_ROOT ||
    hash.startsWith(LEGACY_WORKERS_CONFIGURATION_PREFIX)
  ) {
    return 'configuration'
  }
  if (hash === '#/workers' || hash.startsWith('#/workers/')) {
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

export function hashForView(view: View): string {
  switch (view) {
    case 'traces':
      return '#/traces'
    case 'workers':
      return '#/workers'
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
    const targetHash = hashForView(next)
    if (window.location.hash !== targetHash) {
      window.location.hash = targetHash
    } else {
      setView(next)
    }
  }, [])

  return [view, navigate]
}

/** Swap the hash in place: no history entry, no `hashchange` event. */
export function replaceHash(targetHash: string): void {
  window.history.replaceState(
    window.history.state,
    '',
    `${window.location.pathname}${window.location.search}${targetHash}`,
  )
}

const WORKER_HASH_PREFIX = '#/worker/'

/**
 * `#/worker/<scope>[/<page-id>][?context=<json>]` — the isolated shell's
 * route. `scope` is the worker's asset namespace (the first segment of its
 * script path, the `data-iii-ui` value); the page id picks one of the
 * worker's pages and is omitted for its first. `context` is what
 * `host.panels.open` would have delivered to that page.
 */
export interface WorkerRoute {
  scope: string
  pageId: string | null
  context: JsonValue | null
}

export function workerRouteFromHash(hash: string): WorkerRoute | null {
  if (!hash.startsWith(WORKER_HASH_PREFIX)) return null
  const rest = hash.slice(WORKER_HASH_PREFIX.length)
  const query = rest.indexOf('?')
  const path = query === -1 ? rest : rest.slice(0, query)
  const [scope, pageId] = path.split('/').filter(Boolean).map(decodeSegment)
  if (!scope) return null
  const raw =
    query === -1
      ? null
      : new URLSearchParams(rest.slice(query + 1)).get('context')
  let context: JsonValue | null = null
  if (raw !== null) {
    try {
      context = JSON.parse(raw) as JsonValue
    } catch {
      context = null
    }
  }
  return { scope, pageId: pageId ?? null, context }
}

export function hashForWorkerPage(
  scope: string,
  pageId: string,
  context: JsonValue | null | undefined = null,
): string {
  const base = `${WORKER_HASH_PREFIX}${encodePath([scope, pageId])}`
  if (context === null || context === undefined) return base
  return `${base}?context=${encodeURIComponent(JSON.stringify(context))}`
}

function subscribeHash(listener: () => void): () => void {
  window.addEventListener('hashchange', listener)
  return () => window.removeEventListener('hashchange', listener)
}

/** The live `#/worker/…` route; null outside the isolated shell. */
export function useWorkerRoute(): WorkerRoute | null {
  const hash = useSyncExternalStore(
    subscribeHash,
    () => window.location.hash,
    () => '',
  )
  return useMemo(() => workerRouteFromHash(hash), [hash])
}

export function useWorkersConfigurationRoute(): [
  WorkersConfigurationRoute,
  (configurationId: string | null, fieldPath?: string[]) => void,
] {
  const [route, setRoute] = useState<WorkersConfigurationRoute>(() => {
    if (typeof window === 'undefined') {
      return CLOSED_CONFIGURATION_ROUTE
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
        next.open !== cur.open ||
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

  /** Move within the open surface: to an entry, or (null) back to the
      unselected list — the narrow flow's drill-out. */
  const navigate = useCallback(
    (configurationId: string | null, fieldPath: string[] = []) => {
      const targetHash = configurationId
        ? hashForWorkersConfiguration(configurationId, fieldPath)
        : WORKERS_CONFIGURATION_ROOT
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
