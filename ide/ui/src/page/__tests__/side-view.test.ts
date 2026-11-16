import { describe, expect, it } from 'vitest'
import { selectSideView } from '../side-view'

const wide = { narrow: false }

describe('pressing a sidebar view icon', () => {
  it('Search takes the caret into its box, from the strip or the folded rail', () => {
    // QA 2026-11-16: focus stayed on the icon, so the first Space typed
    // pressed it again and folded the sidebar.
    expect(selectSideView({ view: 'files', collapsed: false }, 'search', { from: 'strip', ...wide })).toEqual({
      view: 'search',
      collapsed: false,
      focusSearch: true,
    })
    expect(selectSideView({ view: 'search', collapsed: true }, 'search', { from: 'rail', ...wide })).toEqual({
      view: 'search',
      collapsed: false,
      focusSearch: true,
    })
  })

  it('the open view pressed again in the strip folds the sidebar, and keeps focus where it is', () => {
    expect(selectSideView({ view: 'search', collapsed: false }, 'search', { from: 'strip', ...wide })).toEqual({
      view: 'search',
      collapsed: true,
      focusSearch: false,
    })
  })

  it('narrow, the sheet never folds from its own strip', () => {
    expect(
      selectSideView({ view: 'search', collapsed: false }, 'search', { from: 'strip', narrow: true }),
    ).toMatchObject({ collapsed: false })
  })

  it('other views open without moving focus', () => {
    expect(selectSideView({ view: 'search', collapsed: false }, 'scm', { from: 'strip', ...wide })).toEqual({
      view: 'scm',
      collapsed: false,
      focusSearch: false,
    })
  })
})
