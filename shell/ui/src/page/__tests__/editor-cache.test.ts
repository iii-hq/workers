import { describe, expect, it } from 'vitest'
import type { EditorCacheEntry } from '../EditorPane'
import {
  currentReviewDirtyPaths,
  refreshCleanEditorCacheEntry,
} from '../editor-cache'

function cacheEntry(
  savedContent: string,
  draft: string,
  revision: string,
): EditorCacheEntry {
  return {
    savedContent,
    draft,
    revision,
    readOnly: null,
    mode: 0o644,
    size: savedContent.length,
  }
}

describe('refreshCleanEditorCacheEntry', () => {
  it('keeps a dirty draft on its original baseline and revision', () => {
    const entry = cacheEntry('before\n', 'my draft\n', 'sha256:before')

    expect(
      refreshCleanEditorCacheEntry(entry, 'external write\n', 'sha256:external'),
    ).toBe(false)
    expect(entry).toMatchObject({
      savedContent: 'before\n',
      draft: 'my draft\n',
      revision: 'sha256:before',
    })
  })

  it('refreshes content and revision when the buffer is clean', () => {
    const entry = cacheEntry('before\n', 'before\n', 'sha256:before')

    expect(
      refreshCleanEditorCacheEntry(entry, 'external write\n', 'sha256:external'),
    ).toBe(true)
    expect(entry).toMatchObject({
      savedContent: 'external write\n',
      draft: 'external write\n',
      revision: 'sha256:external',
    })
  })
})

describe('currentReviewDirtyPaths', () => {
  it('retains a cached review draft after its row reports clean on unmount', () => {
    const cache = new Map([
      ['src/app.ts', cacheEntry('before\n', 'my draft\n', 'sha256:before')],
    ])

    expect(
      currentReviewDirtyPaths(['src/app.ts'], cache, []),
    ).toEqual(new Set(['src/app.ts']))
  })

  it('does not retain a clean cached review session', () => {
    const cache = new Map([
      ['src/app.ts', cacheEntry('before\n', 'before\n', 'sha256:before')],
    ])

    expect(currentReviewDirtyPaths(['src/app.ts'], cache, [])).toEqual(new Set())
  })
})
