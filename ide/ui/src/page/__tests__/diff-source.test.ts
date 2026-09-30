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

  it('commit diffs: parent to commit, fixed, persisted, with hex shas only', () => {
    const source = { type: 'commit', sha: 'abcdef1234', parent: '1234567abc' } as const
    expect(diffSourceKey(source)).toBe('commit=1234567abc..abcdef1234')
    expect(diffSourceLabel(source)).toBe('abcdef1')
    expect(diffSourceSides(source)).toEqual({ old: '1234567', new: 'abcdef1' })
    expect(diffSourceSides({ type: 'commit', sha: 'abcdef1234', parent: null }).old).toBe('empty')
    expect(diffSourceFollowsDisk(source)).toBe(false)
    expect(diffSourcePersists(source)).toBe(true)
    expect(parseDiffSource({ ...source, from: 'old/a.ts' })).toEqual({ ...source, from: 'old/a.ts' })
    expect(parseDiffSource({ type: 'commit', sha: 'abcdef1', parent: null })).toEqual({
      type: 'commit',
      sha: 'abcdef1',
      parent: null,
    })
    expect(parseDiffSource({ type: 'commit', sha: 'main', parent: null })).toBeNull()
    expect(parseDiffSource({ type: 'commit', sha: 'abcdef1' })).toBeNull()
  })

  it('parses persisted sources and rejects junk', () => {
    expect(parseDiffSource({ type: 'turn', turnId: 't1' })).toEqual({ type: 'turn', turnId: 't1' })
    expect(parseDiffSource({ type: 'turn' })).toBeNull()
    expect(parseDiffSource({ type: 'nope' })).toBeNull()
    expect(parseDiffSource('staged')).toBeNull()
  })

  it('commit-panel sources: HEAD to working copy, and a pair of revisions', () => {
    expect(diffSourceKey({ type: 'uncommitted' })).toBe('uncommitted')
    expect(diffSourceLabel({ type: 'uncommitted' })).toBe('Changes')
    expect(diffSourceSides({ type: 'uncommitted' })).toEqual({ old: 'HEAD', new: 'working copy' })
    const revision = { type: 'revision', from: 'p', to: 'c', label: '0d5b60e' } as const
    expect(diffSourceKey(revision)).toBe('revision=p..c')
    expect(diffSourceLabel(revision)).toBe('0d5b60e')
    expect(diffSourceFollowsDisk(revision)).toBe(false)
    expect(diffSourceFollowsDisk({ type: 'uncommitted' })).toBe(true)
    expect(diffSourcePersists(revision)).toBe(true)
    expect(parseDiffSource(revision)).toEqual(revision)
    expect(parseDiffSource({ type: 'revision', from: 'p', to: '', label: 'x' })).toBeNull()
    expect(parseDiffSource({ type: 'uncommitted' })).toEqual({ type: 'uncommitted' })
  })
})
