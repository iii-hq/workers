import { describe, expect, it } from 'vitest'
import type { BrowserRow } from './browser'
import { closestMatches, groupRows, matchesQuery } from './library-model'

const row = (key: string, group?: string, extra: Partial<BrowserRow> = {}): BrowserRow => ({
  key,
  title: key,
  description: '',
  fine: '',
  ...(group !== undefined ? { group } : {}),
  ...extra,
})

describe('groupRows', () => {
  it('puts listed groups first, then named groups A–Z, then ungrouped rows', () => {
    const groups = groupRows(
      [row('a', ''), row('b', 'zeta'), row('c', 'alpha'), row('d', 'yours'), row('e', 'alpha')],
      ['yours'],
      'Standalone',
    )
    expect(groups.map((g) => g.label)).toEqual(['yours', 'alpha', 'zeta', 'Standalone'])
    expect(groups[1].rows.map((r) => r.key)).toEqual(['c', 'e'])
  })

  it('labels a group by the row’s groupLabel when it has one', () => {
    expect(groupRows([row('a', 'kit', { groupLabel: 'From kits' })])[0].label).toBe('From kits')
  })
})

describe('matchesQuery', () => {
  it('matches the key, title or description, case-insensitively', () => {
    const r = row('kanban/tickets/create', 'kanban', { title: 'Create a ticket', description: 'Files a KAN card' })
    expect(matchesQuery(r, 'tickets')).toBe(true)
    expect(matchesQuery(r, 'create a')).toBe(true)
    expect(matchesQuery(r, 'kan card')).toBe(true)
    expect(matchesQuery(r, 'nope')).toBe(false)
    expect(matchesQuery(r, '')).toBe(true)
  })
})

describe('closestMatches', () => {
  const rows = [
    row('ade/console-injectable-ui'),
    row('ade/console-ui-patterns'),
    row('ade/design-console-ui'),
    row('harness/orchestration'),
    row('react'),
  ]

  it('suggests rows sharing a word with a misspelt query', () => {
    expect(closestMatches(rows, 'console-innegne').map((r) => r.key)).toEqual([
      'ade/console-injectable-ui',
      'ade/console-ui-patterns',
      'ade/design-console-ui',
    ])
  })

  it('forgives a typo in a single word', () => {
    expect(closestMatches(rows, 'ochestration').map((r) => r.key)).toEqual(['harness/orchestration'])
  })

  it('suggests nothing for unrelated text or an empty query', () => {
    expect(closestMatches(rows, 'quarterly')).toEqual([])
    expect(closestMatches(rows, '   ')).toEqual([])
  })
})
