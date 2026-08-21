import { beforeEach, describe, expect, it } from 'vitest'
import {
  anyDirty,
  clearTabDirty,
  dirtyReasonsForPane,
  dirtyReasonsForTab,
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
})
