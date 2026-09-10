import { describe, expect, it } from 'vitest'
import {
  formatFileMention,
  formatFileMentionInner,
  formatLineRange,
  normalizeLineRange,
  parseFileMentionInner,
  sameFileMention,
} from './file-mention-token'

describe('parseFileMentionInner', () => {
  it('reads a plain path, a single line and a range', () => {
    expect(parseFileMentionInner('src/a.ts')).toEqual({ path: 'src/a.ts' })
    expect(parseFileMentionInner('src/a.ts:12')).toEqual({
      path: 'src/a.ts',
      range: { from: 12, to: 12 },
    })
    expect(parseFileMentionInner('src/a.ts:12-40')).toEqual({
      path: 'src/a.ts',
      range: { from: 12, to: 40 },
    })
  })

  it('orders a backwards range and keeps spaces in the path', () => {
    expect(parseFileMentionInner('docs/x y.md:40-12')).toEqual({
      path: 'docs/x y.md',
      range: { from: 12, to: 40 },
    })
  })

  it('leaves folders, bare windows and zero lines alone', () => {
    expect(parseFileMentionInner('src/')).toEqual({ path: 'src/' })
    expect(parseFileMentionInner(':12')).toEqual({ path: ':12' })
    expect(parseFileMentionInner('a.ts:0')).toEqual({ path: 'a.ts:0' })
    expect(parseFileMentionInner('  a.ts  ')).toEqual({ path: 'a.ts' })
  })
})

describe('formatting', () => {
  it('writes the inner text and the whole token', () => {
    expect(formatFileMentionInner({ path: 'a.ts' })).toBe('a.ts')
    expect(
      formatFileMentionInner({ path: 'a.ts', range: { from: 3, to: 3 } }),
    ).toBe('a.ts:3')
    expect(formatFileMention({ path: 'a.ts', range: { from: 3, to: 9 } })).toBe(
      '#file(a.ts:3-9)',
    )
    expect(formatLineRange({ from: 1, to: 1 })).toBe('1')
    expect(normalizeLineRange(9, 2)).toEqual({ from: 2, to: 9 })
  })

  it('round-trips through parse', () => {
    for (const inner of ['a.ts', 'a.ts:4', 'src/b c.rs:4-8', 'dir/']) {
      expect(formatFileMentionInner(parseFileMentionInner(inner))).toBe(inner)
    }
  })

  it('compares references by path and window', () => {
    expect(
      sameFileMention(
        { path: 'a.ts', range: { from: 1, to: 2 } },
        { path: 'a.ts', range: { from: 1, to: 2 } },
      ),
    ).toBe(true)
    expect(
      sameFileMention(
        { path: 'a.ts' },
        { path: 'a.ts', range: { from: 1, to: 2 } },
      ),
    ).toBe(false)
  })
})
