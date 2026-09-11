import { describe, expect, it } from 'vitest'
import {
  diffSourceFollowsDisk,
  diffSourceKey,
  diffSourceLabel,
  diffSourcePersists,
  diffSourceSides,
  parseDiffSource,
  sameDiffSource,
} from '../diff-source'

describe('diff-source', () => {
  it('keys, labels and sides', () => {
    expect(diffSourceKey({ type: 'unstaged' })).toBe('unstaged')
    expect(diffSourceLabel({ type: 'compare', ref: 'refs/tags/v1' })).toBe('v1')
    expect(diffSourceLabel({ type: 'turn', turnId: 't' }, 'Fix login')).toBe('Fix login')
    expect(diffSourceSides({ type: 'staged' })).toEqual({ old: 'HEAD', new: 'index' })
    expect(sameDiffSource({ type: 'turn', turnId: 'a' }, { type: 'turn', turnId: 'a' })).toBe(true)
    expect(sameDiffSource({ type: 'turn', turnId: 'a' }, { type: 'turn', turnId: 'b' })).toBe(false)
  })

  it('change diffs neither follow the disk nor persist', () => {
    expect(diffSourceFollowsDisk({ type: 'change', changeId: 'c' })).toBe(false)
    expect(diffSourcePersists({ type: 'change', changeId: 'c' })).toBe(false)
    expect(diffSourceFollowsDisk({ type: 'unstaged' })).toBe(true)
  })

  it('parses persisted sources and rejects junk', () => {
    expect(parseDiffSource({ type: 'turn', turnId: 't1' })).toEqual({ type: 'turn', turnId: 't1' })
    expect(parseDiffSource({ type: 'turn' })).toBeNull()
    expect(parseDiffSource({ type: 'nope' })).toBeNull()
    expect(parseDiffSource('staged')).toBeNull()
  })
})
