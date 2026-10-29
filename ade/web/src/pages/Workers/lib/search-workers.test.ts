import { describe, expect, it } from 'vitest'
import type { ListGroup } from '../model'
import { searchWorkers, splitRanges, suggestWorker } from './search-workers'

const item = (
  name: string,
  detail: string,
  state = 'ready',
  failed = false,
) => ({ name, detail, state, failed, source: null })

const groups: ListGroup[] = [
  {
    id: 'attention',
    label: 'Needs attention',
    items: [item('database', 'exit code 1', 'failed', true)],
  },
  {
    id: 'registry',
    label: 'Registry',
    items: [
      item('provider-github-copilot', '0.1.16'),
      item('provider-openai', '1.2.26'),
      item('judge', '0.2.9'),
      item('judge-typesafe', '0.2.4'),
      item('cron', '0.21.25'),
    ],
  },
  {
    id: 'path',
    label: 'Local path',
    items: [item('ade', 'belo-horizonte/ade', 'starting')],
  },
]

const names = (query: string) =>
  searchWorkers(groups, query).map((hit) => hit.item.name)

describe('searchWorkers', () => {
  it('returns nothing for an empty query', () => {
    expect(searchWorkers(groups, '   ')).toEqual([])
  })

  it('ranks an exact name over a prefix over a word start over a substring', () => {
    expect(names('judge')).toEqual(['judge', 'judge-typesafe'])
    expect(names('open')).toEqual(['provider-openai'])
    expect(names('enai')).toEqual(['provider-openai'])
  })

  it('finds a name by the initials of its words', () => {
    expect(names('pgc')[0]).toBe('provider-github-copilot')
  })

  it('matches the version, state and group when the name does not', () => {
    expect(names('0.2')).toEqual(['cron', 'judge', 'judge-typesafe'])
    expect(names('failed')).toEqual(['database'])
    expect(names('local')).toEqual(['ade'])
  })

  it('needs every token to match somewhere', () => {
    expect(names('judge 0.2.4')).toEqual(['judge-typesafe'])
    expect(names('judge nowhere')).toEqual([])
  })

  it('highlights the matched spans of the name and detail', () => {
    const [hit] = searchWorkers(groups, 'type 0.2')
    expect(hit?.item.name).toBe('judge-typesafe')
    expect(hit?.nameRanges).toEqual([[6, 10]])
    expect(hit?.detailRanges).toEqual([[0, 3]])
  })

  it('keeps the order stable across ties', () => {
    expect(names('provider')).toEqual([
      'provider-github-copilot',
      'provider-openai',
    ])
  })
})

describe('suggestWorker', () => {
  it('offers the nearest name for a typo', () => {
    expect(suggestWorker(groups, 'jugde')).toBe('judge')
    expect(suggestWorker(groups, 'databse')).toBe('database')
  })
  it('stays quiet when nothing is close', () => {
    expect(suggestWorker(groups, 'zzzzzz')).toBeNull()
    expect(suggestWorker(groups, 'ab')).toBeNull()
  })
})

describe('splitRanges', () => {
  it('splits text around the ranges', () => {
    expect(splitRanges('judge-typesafe', [[6, 10]])).toEqual([
      { text: 'judge-', hit: false },
      { text: 'type', hit: true },
      { text: 'safe', hit: false },
    ])
  })
})
