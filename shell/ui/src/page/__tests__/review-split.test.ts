import { describe, expect, it } from 'vitest'
import { wholeFileChange, wholeFileLabel } from '../review-split'

describe('wholeFileChange', () => {
  it('names the side that is missing', () => {
    expect(wholeFileChange('old', '')).toBe('deleted')
    expect(wholeFileChange('', 'new')).toBe('added')
  })

  it('is null when both sides exist or both are empty', () => {
    expect(wholeFileChange('a', 'b')).toBeNull()
    expect(wholeFileChange('', '')).toBeNull()
  })
})

describe('wholeFileLabel', () => {
  it('reads in natural case with a pluralised count', () => {
    expect(wholeFileLabel('deleted', 126)).toEqual({
      title: 'File deleted',
      detail: '126 lines removed',
    })
    expect(wholeFileLabel('added', 1)).toEqual({
      title: 'New file',
      detail: '1 line added',
    })
  })
})
