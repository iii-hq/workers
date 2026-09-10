import { describe, expect, it } from 'vitest'
import { formatFileReference, mentionPathFor, selectionLines } from '../reference'

describe('selectionLines', () => {
  it('covers the lines of a forward selection', () => {
    expect(selectionLines({ startLine: 3, startColumn: 5, endLine: 7, endColumn: 2 })).toEqual({ from: 3, to: 7 })
  })

  it('drops a trailing line the selection only touches at column 1', () => {
    expect(selectionLines({ startLine: 3, startColumn: 1, endLine: 8, endColumn: 1 })).toEqual({ from: 3, to: 7 })
  })

  it('keeps a single line whatever the columns', () => {
    expect(selectionLines({ startLine: 4, startColumn: 1, endLine: 4, endColumn: 1 })).toEqual({ from: 4, to: 4 })
  })

  it('orders a backwards selection', () => {
    expect(selectionLines({ startLine: 9, startColumn: 1, endLine: 2, endColumn: 4 })).toEqual({ from: 2, to: 8 })
  })
})

describe('mentionPathFor', () => {
  it('is relative under the chat folder, absolute elsewhere', () => {
    expect(mentionPathFor('/w/src/a.ts', '/w')).toBe('src/a.ts')
    expect(mentionPathFor('/w/src/a.ts', '/w/')).toBe('src/a.ts')
    expect(mentionPathFor('/other/a.ts', '/w')).toBe('/other/a.ts')
    expect(mentionPathFor('/w-2/a.ts', '/w')).toBe('/w-2/a.ts')
    expect(mentionPathFor('/w/a.ts', null)).toBe('/w/a.ts')
  })
})

describe('formatFileReference', () => {
  it('writes the token with a range or a single line', () => {
    expect(formatFileReference('src/a.ts', { from: 12, to: 40 })).toBe('#file(src/a.ts:12-40)')
    expect(formatFileReference('src/a.ts', { from: 12, to: 12 })).toBe('#file(src/a.ts:12)')
  })
})
