import { describe, expect, it } from 'vitest'
import { changedLineSpan, insertedEnd } from './code-editor-quick-edit'

describe('changedLineSpan', () => {
  it('finds the changed lines between shared top and bottom lines', () => {
    expect(changedLineSpan('a\nb\nc', 'a\nB\nc')).toEqual({ start: 1, end: 1 })
    expect(changedLineSpan('a\nc', 'a\nb1\nb2\nc')).toEqual({ start: 1, end: 2 })
  })

  it('is null when nothing changed or lines were only removed', () => {
    expect(changedLineSpan('a\nb', 'a\nb')).toBeNull()
    expect(changedLineSpan('a\nb\nc', 'a\nc')).toBeNull()
  })

  it('covers everything for a full rewrite', () => {
    expect(changedLineSpan('x', 'y\nz')).toEqual({ start: 0, end: 1 })
  })
})

describe('insertedEnd', () => {
  it('ends on the same line for single-line text', () => {
    expect(insertedEnd(3, 5, 'abc')).toEqual({ line: 3, column: 8 })
  })

  it('ends after the last line of multi-line text', () => {
    expect(insertedEnd(3, 5, 'a\nbc')).toEqual({ line: 4, column: 3 })
    expect(insertedEnd(1, 1, 'a\n')).toEqual({ line: 2, column: 1 })
  })
})
