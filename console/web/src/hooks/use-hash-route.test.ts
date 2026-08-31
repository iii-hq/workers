import { describe, expect, it } from 'vitest'
import {
  extPageFromHash,
  hashForWorkersConfiguration,
  normalizeExtHash,
  normalizeWorkersConfigurationHash,
  routeFromHash,
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

describe('migrated-page hash redirects', () => {
  it('rewrites legacy first-party hashes to their injected #/ext/<id> route', () => {
    expect(normalizeExtHash('#/worktrees')).toBe('#/ext/worktree')
    expect(normalizeExtHash('#/memory')).toBe('#/ext/memory')
    expect(normalizeExtHash('#/browser')).toBe('#/ext/browser')
    expect(normalizeExtHash('#/github')).toBe('#/ext/github')
  })

  it('resolves the injected page id from a legacy hash', () => {
    expect(extPageFromHash(normalizeExtHash('#/worktrees'))).toBe('worktree')
    expect(extPageFromHash(normalizeExtHash('#/memory'))).toBe('memory')
    expect(extPageFromHash(normalizeExtHash('#/browser'))).toBe('browser')
    expect(extPageFromHash(normalizeExtHash('#/github'))).toBe('github')
  })

  it('passes non-migrated hashes through unchanged', () => {
    expect(normalizeExtHash('#/traces')).toBe('#/traces')
    expect(normalizeExtHash('#/ext/database')).toBe('#/ext/database')
  })
})
