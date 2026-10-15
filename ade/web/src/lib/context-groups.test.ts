// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { mergeOrder, moveSection, resetFoldedGroups } from './context-groups'

const kept = (): string[] =>
  JSON.parse(window.localStorage.getItem('iii-context-order') ?? '[]')

beforeEach(() => {
  window.localStorage.clear()
  resetFoldedGroups()
})

describe('context section order', () => {
  it('puts each missing id back after the one it followed', () => {
    expect(
      mergeOrder(['links', 'project'], ['project', 'tickets', 'links']),
    ).toEqual(['links', 'project', 'tickets'])
    expect(mergeOrder(['b'], ['a', 'b'])).toEqual(['a', 'b'])
  })

  it("keeps a worker's place while its section is away", () => {
    window.localStorage.setItem(
      'iii-context-order',
      JSON.stringify(['project', 'sentinel-errors', 'links']),
    )
    resetFoldedGroups()
    // Sentinel is not connected: the panel shows Project and Links.
    moveSection(['project', 'links'], 'links', { before: 'project' })
    expect(kept()).toEqual(['links', 'project', 'sentinel-errors'])
  })

  it('moves after a neighbour', () => {
    moveSection(['project', 'tickets', 'links'], 'project', { after: 'links' })
    expect(kept()).toEqual(['tickets', 'links', 'project'])
  })
})
