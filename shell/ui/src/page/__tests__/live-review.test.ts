import { describe, expect, it } from 'vitest'
import { normalizeLiveReviewEvent } from '../live-review'

describe('normalizeLiveReviewEvent', () => {
  it('ignores a deleted path that was a directory', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/generated',
        rawKind: 'deleted',
        priorKind: 'dir',
        existsNow: false,
      }),
    ).toEqual({ action: 'ignore-directory', path: 'src/generated' })
  })

  it('drops a transient file that was created and deleted within the burst', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/.swap-file',
        rawKind: 'deleted',
        priorKind: null,
        existsNow: false,
      }),
    ).toEqual({ action: 'ignore-delete', path: 'src/.swap-file' })
  })

  it('treats a created event for an existing file as an atomic replace', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/app.ts',
        rawKind: 'created',
        priorKind: 'file',
        priorBaseline: 'const version = 1\n',
        existsNow: true,
      }),
    ).toEqual({
      action: 'modified',
      path: 'src/app.ts',
      baseline: 'const version = 1\n',
    })
  })

  it('classifies a truly new file as created against an empty baseline', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/new.ts',
        rawKind: 'created',
        priorKind: null,
        existsNow: true,
      }),
    ).toEqual({ action: 'created', path: 'src/new.ts', baseline: '' })
  })

  it('trusts an explicit created event when no tree snapshot exists yet', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/early.ts',
        rawKind: 'created',
        priorKind: undefined,
        existsNow: true,
      }),
    ).toEqual({ action: 'created', path: 'src/early.ts', baseline: '' })
  })

  it('still recognizes a new file when the watcher only reports its final write', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/generated.ts',
        rawKind: 'modified',
        priorKind: null,
        existsNow: true,
      }),
    ).toEqual({ action: 'created', path: 'src/generated.ts', baseline: '' })
  })

  it('ignores a deleted path that was already missing', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'deep/removed.ts',
        rawKind: 'deleted',
        priorKind: null,
        existsNow: false,
      }),
    ).toEqual({ action: 'ignore-delete', path: 'deep/removed.ts' })
  })

  it('keeps ordinary file modifications and deletions distinct', () => {
    const input = {
      path: 'src/existing.ts',
      priorKind: 'file' as const,
      priorBaseline: 'before\n',
    }

    expect(normalizeLiveReviewEvent({ ...input, rawKind: 'modified', existsNow: true })).toEqual({
      action: 'modified',
      path: 'src/existing.ts',
      baseline: 'before\n',
    })
    expect(normalizeLiveReviewEvent({ ...input, rawKind: 'deleted', existsNow: false })).toEqual({
      action: 'deleted',
      path: 'src/existing.ts',
      baseline: 'before\n',
    })
  })

  it('uses current existence when an old-path rename ends with a modified event', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/old-name.ts',
        rawKind: 'modified',
        priorKind: 'file',
        priorBaseline: 'original\n',
        existsNow: false,
      }),
    ).toEqual({
      action: 'deleted',
      path: 'src/old-name.ts',
      baseline: 'original\n',
    })
  })

  it('believes a witnessed creation over an inventory that only guessed', () => {
    // A truncated inventory answers "file" for every path it never listed, so
    // a file the turn created would otherwise read as modified with nothing
    // to compare against.
    expect(
      normalizeLiveReviewEvent({
        path: 'ReachAI/Dockerfile',
        rawKind: 'created',
        priorKind: 'file',
        priorKindExact: false,
        existsNow: true,
      }),
    ).toEqual({
      action: 'created',
      path: 'ReachAI/Dockerfile',
      baseline: '',
    })
  })

  it('keeps a proven existing file a modification, however it was written', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/atomic.ts',
        rawKind: 'created',
        priorKind: 'file',
        priorKindExact: true,
        existsNow: true,
      }),
    ).toEqual({
      action: 'modified',
      path: 'src/atomic.ts',
      baseline: undefined,
    })
  })

  it('keeps a captured body even when the classification was a guess', () => {
    expect(
      normalizeLiveReviewEvent({
        path: 'src/guessed.ts',
        rawKind: 'created',
        priorKind: 'file',
        priorKindExact: false,
        priorBaseline: 'before\n',
        existsNow: true,
      }),
    ).toEqual({
      action: 'modified',
      path: 'src/guessed.ts',
      baseline: 'before\n',
    })
  })
})
