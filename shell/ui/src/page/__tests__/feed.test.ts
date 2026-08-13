import { describe, expect, it } from 'vitest'
import { lineDelta, splitLines } from '../feed'

describe('splitLines', () => {
  it('treats empty text as zero lines', () => {
    expect(splitLines('')).toEqual([])
    expect(splitLines('a')).toEqual(['a'])
    expect(splitLines('a\nb')).toEqual(['a', 'b'])
  })

  it('counts a trailing newline as a terminator, not a new line', () => {
    expect(splitLines('a\nb\n')).toEqual(['a', 'b'])
    expect(splitLines('\n')).toEqual([])
  })
})

describe('lineDelta', () => {
  it('counts a pure append', () => {
    expect(lineDelta('a\nb', 'a\nb\nc\nd')).toEqual({ add: 2, del: 0 })
  })

  it('counts a pure prepend', () => {
    expect(lineDelta('a\nb', 'x\na\nb')).toEqual({ add: 1, del: 0 })
  })

  it('counts a contiguous replacement', () => {
    expect(lineDelta('a\nOLD\nz', 'a\nNEW1\nNEW2\nz')).toEqual({ add: 2, del: 1 })
  })

  it('counts full creation and deletion', () => {
    expect(lineDelta('', 'a\nb\nc')).toEqual({ add: 3, del: 0 })
    expect(lineDelta('a\nb\nc', '')).toEqual({ add: 0, del: 3 })
  })

  it('reports zero for identical text', () => {
    expect(lineDelta('a\nb', 'a\nb')).toEqual({ add: 0, del: 0 })
  })

  it('never double-counts when prefix and suffix overlap', () => {
    // Old is entirely a prefix AND suffix of new — head consumes it
    // first; tail must stop at what's left.
    expect(lineDelta('a', 'a\na')).toEqual({ add: 1, del: 0 })
  })
})
