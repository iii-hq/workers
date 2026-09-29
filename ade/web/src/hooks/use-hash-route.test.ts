import { describe, expect, it } from 'vitest'
import {
  hashForSettingsLanding,
  hashForStandalone,
  hashForWorkerPage,
  hashForWorkersConfiguration,
  normalizeWorkersConfigurationHash,
  routeFromHash,
  standaloneRouteFromHash,
  workerRouteFromHash,
  workersConfigurationRouteFromHash,
} from './use-hash-route'

describe('workers configuration hash helpers', () => {
  it('builds canonical workers configuration hashes', () => {
    expect(hashForWorkersConfiguration('llm-router')).toBe(
      '#/configuration/workers/llm-router',
    )
    expect(hashForWorkersConfiguration('shell', ['fs', 'host_roots'])).toBe(
      '#/configuration/workers/shell/fs/host_roots',
    )
  })

  it('opens settings on the navigation menu only on narrow screens', () => {
    expect(hashForSettingsLanding(true)).toBe('#/configuration/workers')
    expect(hashForSettingsLanding(false)).toBe('#/configuration')
  })

  it('parses canonical workers configuration hashes', () => {
    expect(
      workersConfigurationRouteFromHash(
        '#/configuration/workers/shell/fs/host_roots',
      ),
    ).toEqual({
      open: true,
      configurationId: 'shell',
      fieldPath: ['fs', 'host_roots'],
    })
  })

  it('parses the bare configuration root as open with no selection', () => {
    expect(
      workersConfigurationRouteFromHash('#/configuration/workers'),
    ).toEqual({
      open: true,
      configurationId: null,
      fieldPath: [],
    })
    expect(
      workersConfigurationRouteFromHash('#/configuration/workers/'),
    ).toEqual({
      open: true,
      configurationId: null,
      fieldPath: [],
    })
  })

  it('parses legacy workers configuration hashes for compatibility', () => {
    expect(
      workersConfigurationRouteFromHash(
        '#/workers/configuration/llm-router/providers/openai',
      ),
    ).toEqual({
      open: true,
      configurationId: 'llm-router',
      fieldPath: ['providers', 'openai'],
    })
  })

  it('normalizes legacy worker configuration hashes', () => {
    expect(normalizeWorkersConfigurationHash('#/workers/configuration')).toBe(
      '#/configuration/workers',
    )
    expect(normalizeWorkersConfigurationHash('#/workers/configuration/')).toBe(
      '#/configuration/workers',
    )
    expect(
      normalizeWorkersConfigurationHash(
        '#/workers/configuration/shell/fs/host_roots',
      ),
    ).toBe('#/configuration/workers/shell/fs/host_roots')
    expect(
      normalizeWorkersConfigurationHash('#/configuration/workers/shell'),
    ).toBeNull()
  })

  it('returns a closed route for non-configuration hashes', () => {
    expect(workersConfigurationRouteFromHash('#/workers')).toEqual({
      open: false,
      configurationId: null,
      fieldPath: [],
    })
  })

  it('routes canonical and legacy configuration hashes to settings', () => {
    expect(routeFromHash('#/configuration/workers/browser')).toBe(
      'configuration',
    )
    expect(routeFromHash('#/workers/configuration/browser')).toBe(
      'configuration',
    )
    expect(routeFromHash('#/workers')).toBe('workers')
  })
})

describe('isolated worker route', () => {
  it('parses scope, page id and context', () => {
    expect(workerRouteFromHash('#/worker/state')).toEqual({
      scope: 'state',
      pageId: null,
      context: null,
    })
    expect(workerRouteFromHash('#/worker/kanban/ticket')).toEqual({
      scope: 'kanban',
      pageId: 'ticket',
      context: null,
    })
    expect(
      workerRouteFromHash(
        '#/worker/kanban/ticket?context=%7B%22id%22%3A%22T-1%22%7D',
      ),
    ).toEqual({ scope: 'kanban', pageId: 'ticket', context: { id: 'T-1' } })
  })

  it('round-trips through hashForWorkerPage, context included', () => {
    const context = { id: 'a+b', nested: [1, { ok: true }] }
    expect(workerRouteFromHash(hashForWorkerPage('x', 'p', context))).toEqual({
      scope: 'x',
      pageId: 'p',
      context,
    })
    expect(hashForWorkerPage('x', 'p')).toBe('#/worker/x/p')
    expect(hashForWorkerPage('x', 'p', null)).toBe('#/worker/x/p')
    expect(hashForWorkerPage('x', null)).toBe('#/worker/x')
  })

  it('names the standalone surfaces: a worker page or the traces explorer', () => {
    expect(standaloneRouteFromHash('#/traces')).toEqual({ kind: 'traces' })
    expect(standaloneRouteFromHash('#/worker/ide')).toEqual({
      kind: 'worker',
      scope: 'ide',
      pageId: null,
      context: null,
    })
    expect(standaloneRouteFromHash('#/workers')).toBeNull()
    expect(standaloneRouteFromHash('#/configuration')).toBeNull()
    expect(standaloneRouteFromHash('#/')).toBeNull()
    expect(hashForStandalone({ kind: 'traces' })).toBe('#/traces')
    expect(
      hashForStandalone({
        kind: 'worker',
        scope: 'ide',
        pageId: 'ide',
        context: { a: 1 },
      }),
    ).toBe(hashForWorkerPage('ide', 'ide', { a: 1 }))
  })

  it('tolerates a malformed context and rejects other hashes', () => {
    expect(workerRouteFromHash('#/worker/x/p?context=nope')).toEqual({
      scope: 'x',
      pageId: 'p',
      context: null,
    })
    expect(workerRouteFromHash('#/worker/')).toBeNull()
    expect(workerRouteFromHash('#/workers')).toBeNull()
    expect(workerRouteFromHash('#/ext/state')).toBeNull()
    // The app's own router never claims it either: the shell is chosen at boot.
    expect(routeFromHash('#/worker/state')).toBeNull()
  })
})
