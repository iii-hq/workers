import { describe, expect, it } from 'vitest'
import { diffLines } from '../diff'
import {
  firstChangedLine,
  gutterLineFromPath,
  newLineForOld,
  resolveEditorLine,
} from '../open-line'

const cellOf = (dataset: Record<string, string>) => ({ dataset }) as unknown as EventTarget

describe('gutterLineFromPath', () => {
  it('reads the first gutter cell on the composed path and its side', () => {
    const cell = cellOf({ columnNumber: '42', lineType: 'addition' })
    const deleted = cellOf({ columnNumber: '7', lineType: 'deletion' })
    const text = cellOf({ lineNumberContent: '' })
    expect(gutterLineFromPath([text, cell])).toEqual({ line: 42, side: 'new' })
    expect(gutterLineFromPath([deleted])).toEqual({ line: 7, side: 'old' })
    expect(gutterLineFromPath([text])).toBeNull()
    expect(gutterLineFromPath([])).toBeNull()
  })

  it('rejects cells without a usable number', () => {
    const blank = cellOf({ columnNumber: '', lineType: 'context' })
    expect(gutterLineFromPath([blank])).toBeNull()
  })
})

describe('line resolution against the diff', () => {
  const ops = diffLines('a\nb\nc\nd\ne\n', 'a\nX\nc\ne\nY\n')

  it('maps old lines to the working file', () => {
    expect(newLineForOld(ops, 1)).toBe(1)
    expect(newLineForOld(ops, 2)).toBe(2)
    expect(newLineForOld(ops, 3)).toBe(3)
    expect(newLineForOld(ops, 4)).toBe(4)
    expect(newLineForOld(ops, 5)).toBe(4)
    expect(newLineForOld(ops, 99)).toBe(6)
  })

  it('finds the first change and resolves gutter targets by side', () => {
    expect(firstChangedLine(ops)).toBe(2)
    expect(firstChangedLine(diffLines('same\n', 'same\n'))).toBe(1)
    expect(resolveEditorLine(ops, { line: 5, side: 'new' })).toBe(5)
    expect(resolveEditorLine(ops, { line: 4, side: 'old' })).toBe(4)
  })
})
