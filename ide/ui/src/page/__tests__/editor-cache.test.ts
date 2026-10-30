import { describe, expect, it } from 'vitest'
import type { EditorCacheEntry } from '../EditorPane'
import { refreshCleanEditorCacheEntry } from '../editor-cache'

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

describe('surviving a remount', () => {
  const entry = (savedContent: string, draft: string) => ({
    savedContent,
    draft,
    readOnly: null,
    mode: null,
    size: null,
    image: 'blob:x',
  })

  it('keeps only unsaved buffers, and hands them back once', async () => {
    const { keepUnsavedEdits, takeUnsavedEdits } = await import('../editor-cache')
    const cache = new Map([
      ['a.ts', entry('a', 'a edited')],
      ['b.ts', entry('b', 'b')],
    ])
    keepUnsavedEdits('pane-1', '/r', cache as never)
    const back = takeUnsavedEdits('pane-1', '/r')
    expect([...back.keys()]).toEqual(['a.ts'])
    expect(back.get('a.ts')?.draft).toBe('a edited')
    // An image URL was revoked with the old page.
    expect(back.get('a.ts')?.image).toBeNull()
    expect(takeUnsavedEdits('pane-1', '/r').size).toBe(0)
  })

  it('hands edits back only to the folder they were made in', async () => {
    const { keepUnsavedEdits, takeUnsavedEdits } = await import('../editor-cache')
    keepUnsavedEdits('pane-2', '/worktree-a', new Map([['src/x.ts', entry('x', 'x edited')]]) as never)
    // The pane came back on another worktree: its src/x.ts is another file.
    expect(takeUnsavedEdits('pane-2', '/worktree-b').size).toBe(0)
    // Another pane on the same folder holds its own buffers.
    expect(takeUnsavedEdits('pane-3', '/worktree-a').size).toBe(0)
    // Back on the first worktree, the draft is still there.
    expect(takeUnsavedEdits('pane-2', '/worktree-a').get('src/x.ts')?.draft).toBe('x edited')
  })

  it('rejoins kept edits with a tab each, keeping the restored active tab', async () => {
    const { rejoinKeptEdits } = await import('../editor-cache')
    const { restoreTabs } = await import('../tabs')
    const cache = new Map()
    const kept = new Map([
      ['a.ts', entry('a', 'a edited')],
      ['gone-from-tabs.ts', entry('g', 'g edited')],
    ])
    const restored = restoreTabs([{ path: 'a.ts' }, { path: 'b.ts' }], 'file:b.ts')
    const { tabs, dirty } = rejoinKeptEdits(kept as never, cache, restored)
    expect([...cache.keys()]).toEqual(['a.ts', 'gone-from-tabs.ts'])
    expect(tabs.tabs.map((tab) => tab.id)).toEqual(['file:a.ts', 'file:b.ts', 'file:gone-from-tabs.ts'])
    expect(tabs.tabs.at(-1)?.pinned).toBe(true)
    expect(tabs.active).toBe('file:b.ts')
    expect([...dirty]).toEqual(['a.ts', 'gone-from-tabs.ts'])
    // Nothing kept: the restored tabs as they were.
    const none = rejoinKeptEdits(new Map(), new Map(), restored)
    expect(none.tabs).toBe(restored)
    expect(none.dirty.size).toBe(0)
  })

  it('remembers the place per file, newest last', async () => {
    const { rememberViewState, rememberedViewState } = await import('../editor-cache')
    rememberViewState('/r/a.ts', { top: 1 })
    rememberViewState('/r/a.ts', { top: 2 })
    expect(rememberedViewState('/r/a.ts')).toEqual({ top: 2 })
    expect(rememberedViewState('/r/none.ts')).toBeUndefined()
  })
})
