import { describe, expect, it } from 'vitest'
import { type DiffOp, diffLines, diffTotals, foldRows } from '../diff'

const kinds = (ops: DiffOp[]) => ops.map((o) => o.type).join(' ')

describe('diffLines', () => {
  it('reports identical text as all same with line numbers', () => {
    const ops = diffLines('a\nb', 'a\nb')
    expect(kinds(ops)).toBe('same same')
    expect(ops[1]).toMatchObject({ oldLine: 2, newLine: 2 })
  })

  it('finds a contiguous replacement with del before add', () => {
    const ops = diffLines('a\nOLD\nz', 'a\nNEW\nz')
    expect(kinds(ops)).toBe('same del add same')
    expect(ops[1]).toMatchObject({ text: 'OLD', oldLine: 2 })
    expect(ops[2]).toMatchObject({ text: 'NEW', newLine: 2 })
  })

  it('handles pure creation and deletion', () => {
    expect(kinds(diffLines('', 'a\nb'))).toBe('add add')
    expect(kinds(diffLines('a\nb', ''))).toBe('del del')
  })

  it('recovers an interleaved edit through the LCS', () => {
    const ops = diffLines('a\nb\nc\nd', 'a\nx\nc\ny\nd')
    expect(diffTotals(ops)).toEqual({ add: 2, del: 1 })
    const sames = ops.filter((o) => o.type === 'same').map((o) => o.text)
    expect(sames).toEqual(['a', 'c', 'd'])
  })

  it('falls back to a coarse replace past the edit-distance budget', () => {
    // 500 fully different lines each side: D ≈ 1000 blows the budget —
    // the result must be the whole middle as del-run then add-run, not
    // an O(D·N)-memory backtrack.
    const olds = Array.from({ length: 500 }, (_, i) => `old-${String(i)}`).join('\n')
    const news = Array.from({ length: 500 }, (_, i) => `new-${String(i)}`).join('\n')
    const ops = diffLines(olds, news)
    expect(diffTotals(ops)).toEqual({ add: 500, del: 500 })
    expect(ops.slice(0, 500).every((o) => o.type === 'del')).toBe(true)
    expect(ops.slice(500).every((o) => o.type === 'add')).toBe(true)
  })

  it('marks the changed char span inside a replaced line pair', () => {
    const ops = diffLines('const value = 1', 'const value = 2')
    const del = ops.find((o) => o.type === 'del')
    const add = ops.find((o) => o.type === 'add')
    expect(del?.hl).toEqual([14, 15])
    expect(add?.hl).toEqual([14, 15])
  })
})

describe('foldRows', () => {
  const same = (n: number): string => Array.from({ length: n }, (_, i) => `l${String(i)}`).join('\n')

  it('folds a long interior run keeping context on both sides', () => {
    const oldText = `X\n${same(20)}\nY`
    const newText = `X2\n${same(20)}\nY2`
    const rows = foldRows(diffLines(oldText, newText), 3)
    const fold = rows.find((r) => r.kind === 'fold')
    expect(fold).toBeDefined()
    expect(fold?.kind === 'fold' && fold.count).toBe(20 - 6)
  })

  it('keeps short runs whole', () => {
    const rows = foldRows(diffLines('a\nb\nX\nc', 'a\nb\nY\nc'), 3)
    expect(rows.every((r) => r.kind === 'op')).toBe(true)
  })

  it('folds top and bottom runs to a single context edge', () => {
    const text = `${same(30)}\nMID\n${same(30, )}`
    const changed = text.replace('MID', 'MID2')
    const rows = foldRows(diffLines(text, changed), 3)
    const folds = rows.filter((r) => r.kind === 'fold')
    expect(folds).toHaveLength(2)
    // Leading fold keeps only trailing context: 30 - 3 hidden.
    expect(folds[0].kind === 'fold' && folds[0].count).toBe(27)
  })
})
