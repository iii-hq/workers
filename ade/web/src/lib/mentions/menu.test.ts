import { describe, expect, it } from 'vitest'
import { type MentionCandidate, mentionKey } from '@/lib/mention-search'
import {
  buildGlobalMenu,
  buildScopedMenu,
  GLOBAL_PROVIDER_ROWS,
  matchingProviders,
  parseScopedQuery,
  scopedText,
  showsSectionHeaders,
} from './menu'
import type { ProviderResults } from './search'
import type { MentionItem, MentionProvider } from './types'

function provider(name: string, label: string): MentionProvider {
  return {
    v: 1,
    name,
    label,
    search: `${name}::mention::search`,
    getFunctionId: `${name}::mention::get`,
  }
}

const kanban = provider('kanban', 'Tickets')
const session = provider('session', 'Sessions')
const providers = [kanban, session]
const byName = new Map(providers.map((p) => [p.name, p]))

function items(n: number, prefix = 't'): MentionItem[] {
  return Array.from({ length: n }, (_, i) => ({
    id: `${prefix}${i}`,
    label: `item ${i}`,
  }))
}

function results(
  entries: Record<string, MentionItem[]>,
): ReadonlyMap<string, ProviderResults> {
  return new Map(
    Object.entries(entries).map(([name, list]) => [
      name,
      { items: list, loading: false, failed: false },
    ]),
  )
}

const fn: MentionCandidate = {
  kind: 'function',
  id: 'kanban::ticket::get',
  description: 'read a ticket',
}

describe('parseScopedQuery', () => {
  it('scopes to a known provider and keeps spaces in the text', () => {
    expect(parseScopedQuery('kanban:fix login', byName)).toEqual({
      provider: kanban,
      query: 'fix login',
    })
    expect(parseScopedQuery('kanban:', byName)).toEqual({
      provider: kanban,
      query: '',
    })
  })

  it('never scopes a function id or an unknown name', () => {
    expect(parseScopedQuery('kanban::ticket::get', byName)).toBeNull()
    expect(parseScopedQuery('calendar:today', byName)).toBeNull()
    expect(parseScopedQuery('kanban', byName)).toBeNull()
  })

  it('writes the scoping text back', () => {
    expect(scopedText(kanban)).toBe('@kanban:')
    expect(scopedText(kanban, 'fix')).toBe('@kanban:fix')
  })
})

describe('matchingProviders', () => {
  it('matches the name or the label by prefix, all for no text', () => {
    expect(matchingProviders('', providers)).toEqual(providers)
    expect(matchingProviders('kan', providers)).toEqual([kanban])
    expect(matchingProviders('Tick', providers)).toEqual([kanban])
    expect(matchingProviders('kanban::', providers)).toEqual([])
  })
})

describe('buildGlobalMenu', () => {
  const base = {
    providers,
    catalog: [fn],
    catalogRemaining: 0,
    catalogKey: mentionKey,
    results: results({}),
  }

  it('lists matching sources first, then the catalog, then provider groups', () => {
    const entries = buildGlobalMenu({
      ...base,
      query: 'kan',
      results: results({ kanban: items(2), session: items(1, 's') }),
    })
    expect(entries.map((e) => `${e.section.key}/${e.row.kind}`)).toEqual([
      'sources/provider',
      'catalog/candidate',
      'provider:kanban/entity',
      'provider:kanban/entity',
      'provider:session/entity',
    ])
  })

  it('offers "more" only when a provider returned more than it shows', () => {
    const entries = buildGlobalMenu({
      ...base,
      query: 'item',
      results: results({ kanban: items(GLOBAL_PROVIDER_ROWS + 1) }),
    })
    const kanbanRows = entries.filter(
      (e) => e.section.key === 'provider:kanban',
    )
    expect(kanbanRows.map((e) => e.row.kind)).toEqual([
      'entity',
      'entity',
      'entity',
      'provider-more',
    ])
    expect(kanbanRows.at(-1)?.row).toMatchObject({ query: 'item' })
  })

  it('asks no provider groups below two characters', () => {
    const entries = buildGlobalMenu({
      ...base,
      query: 'k',
      results: results({ kanban: items(2) }),
    })
    expect(entries.some((e) => e.row.kind === 'entity')).toBe(false)
  })

  it('keeps the catalog pager', () => {
    const entries = buildGlobalMenu({
      ...base,
      query: 'zz',
      catalogRemaining: 4,
    })
    expect(entries.at(-1)?.row).toEqual({ kind: 'more', remaining: 4 })
  })

  it('draws headers for several groups or a lone worker group', () => {
    const catalogOnly = buildGlobalMenu({ ...base, query: 'zz' })
    expect(showsSectionHeaders(catalogOnly)).toBe(false)
    expect(
      showsSectionHeaders(buildGlobalMenu({ ...base, query: 'kan' })),
    ).toBe(true)
    const loneGroup = buildGlobalMenu({
      ...base,
      catalog: [],
      query: 'fix',
      results: results({ kanban: items(1) }),
    })
    expect(showsSectionHeaders(loneGroup)).toBe(true)
  })
})

describe('buildScopedMenu', () => {
  it('lists only that provider, with stable keys', () => {
    const entries = buildScopedMenu(
      { provider: kanban, query: 'fix' },
      results({ kanban: items(2), session: items(3, 's') }),
    )
    expect(entries.map((e) => e.key)).toEqual([
      'entity:kanban:t0',
      'entity:kanban:t1',
    ])
  })
})
