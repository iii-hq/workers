import { beforeEach, describe, expect, it } from 'vitest'
import {
  anyDirty,
  clearTabDirty,
  dirtyReasonsForPane,
  dirtyReasonsForTab,
  pruneDirty,
  resetDirtyRegistry,
  setPaneDirty,
} from './workspace-guards'

describe('workspace dirty registry', () => {
  beforeEach(() => resetDirtyRegistry())

  it('collects reasons per tab and per pane', () => {
    setPaneDirty('tab-1', 'pane-a', 'main.rs')
    setPaneDirty('tab-1', 'pane-b', 'notes.md')
    setPaneDirty('tab-2', 'pane-c', 'Unsaved changes')
    expect(dirtyReasonsForTab('tab-1')).toEqual(['main.rs', 'notes.md'])
    expect(dirtyReasonsForPane('tab-1', 'pane-b')).toEqual(['notes.md'])
    expect(dirtyReasonsForTab('tab-3')).toEqual([])
    expect(anyDirty()).toBe(true)
  })

  it('a pane marked clean or a cleared tab drops out', () => {
    setPaneDirty('tab-1', 'pane-a', 'main.rs')
    setPaneDirty('tab-1', 'pane-b', 'notes.md')
    setPaneDirty('tab-1', 'pane-a', false)
    expect(dirtyReasonsForTab('tab-1')).toEqual(['notes.md'])
    clearTabDirty('tab-1')
    expect(dirtyReasonsForTab('tab-1')).toEqual([])
    expect(anyDirty()).toBe(false)
  })

  it('re-reporting the same reason keeps one entry', () => {
    setPaneDirty('tab-1', 'pane-a', 'main.rs')
    setPaneDirty('tab-1', 'pane-a', 'main.rs')
    expect(dirtyReasonsForTab('tab-1')).toEqual(['main.rs'])
  })

  it('pruning keeps only panes that still exist in the layout', () => {
    setPaneDirty('tab-1', 'pane-a', 'main.rs')
    setPaneDirty('tab-1', 'pane-b', 'notes.md')
    setPaneDirty('tab-2', 'pane-c', 'Unsaved changes')
    pruneDirty(new Set(['tab-1']), new Map([['tab-1', new Set(['pane-a'])]]))
    expect(dirtyReasonsForTab('tab-1')).toEqual(['main.rs'])
    expect(dirtyReasonsForTab('tab-2')).toEqual([])
  })

  it('ids with separators never collide', () => {
    setPaneDirty('a b', 'c', 'one')
    setPaneDirty('a', 'b c', 'two')
    expect(dirtyReasonsForTab('a b')).toEqual(['one'])
    expect(dirtyReasonsForTab('a')).toEqual(['two'])
  })
})
