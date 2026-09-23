import { describe, expect, it } from 'vitest'
import { referenceWarning } from '../reference-warning'

describe('referenceWarning', () => {
  it('allows valid inclusive ranges and end-of-line columns', () => {
    expect(referenceWarning({ line: 1, endLine: 2 }, 'abc\ndef', null)).toBeNull()
    expect(referenceWarning({ line: 1, column: 4 }, 'abc', null)).toBeNull()
  })
  it('explains missing lines, stale columns, previews and binary files', () => {
    expect(referenceWarning({ line: 3 }, 'a\nb', null)).toContain('may have changed')
    expect(referenceWarning({ line: 1, endLine: 9 }, 'a\nb', null)).toContain('exceeds')
    expect(referenceWarning({ line: 1, column: 5 }, 'abc', null)).toContain('column')
    expect(referenceWarning({ line: 5000 }, 'preview', 'truncated')).toContain('not been loaded in full')
    expect(referenceWarning({ line: 1 }, '', 'binary')).toContain('binary')
  })
})
