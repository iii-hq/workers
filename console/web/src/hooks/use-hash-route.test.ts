import { describe, expect, it } from 'vitest'
import {
  extPageFromHash,
  hashForWorkersConfiguration,
  normalizeExtHash,
  normalizeWorkersConfigurationHash,
  workersConfigurationRouteFromHash,
} from './use-hash-route'

describe('workers configuration hash helpers', () => {
  it('builds canonical workers configuration hashes', () => {
    expect(hashForWorkersConfiguration('llm-router')).toBe(
      '#/workers/configuration/llm-router',
    )
    expect(hashForWorkersConfiguration('shell', ['fs', 'host_roots'])).toBe(
      '#/workers/configuration/shell/fs/host_roots',
    )
  })

  it('parses canonical workers configuration hashes', () => {
    expect(
      workersConfigurationRouteFromHash(
        '#/workers/configuration/shell/fs/host_roots',
      ),
    ).toEqual({
      configurationId: 'shell',
      fieldPath: ['fs', 'host_roots'],
    })
  })

  it('parses legacy configuration worker hashes for compatibility', () => {
    expect(
      workersConfigurationRouteFromHash(
        '#/configuration/workers/llm-router/providers/openai',
      ),
    ).toEqual({
      configurationId: 'llm-router',
      fieldPath: ['providers', 'openai'],
    })
  })

  it('normalizes legacy worker configuration hashes', () => {
    expect(normalizeWorkersConfigurationHash('#/configuration/workers')).toBe(
      '#/workers',
    )
    expect(
      normalizeWorkersConfigurationHash(
        '#/configuration/workers/shell/fs/host_roots',
      ),
    ).toBe('#/workers/configuration/shell/fs/host_roots')
    expect(
      normalizeWorkersConfigurationHash('#/workers/configuration/shell'),
    ).toBeNull()
  })

  it('returns an empty route for non-configuration hashes', () => {
    expect(workersConfigurationRouteFromHash('#/workers')).toEqual({
      configurationId: null,
      fieldPath: [],
    })
  })
})

describe('migrated-page hash redirects', () => {
  it('rewrites legacy first-party hashes to their injected #/ext/<id> route', () => {
    expect(normalizeExtHash('#/worktrees')).toBe('#/ext/worktree')
    expect(normalizeExtHash('#/memory')).toBe('#/ext/memory')
  })

  it('resolves the injected page id from a legacy hash', () => {
    expect(extPageFromHash(normalizeExtHash('#/worktrees'))).toBe('worktree')
    expect(extPageFromHash(normalizeExtHash('#/memory'))).toBe('memory')
  })

  it('passes non-migrated hashes through unchanged', () => {
    expect(normalizeExtHash('#/traces')).toBe('#/traces')
    expect(normalizeExtHash('#/ext/database')).toBe('#/ext/database')
  })
})
