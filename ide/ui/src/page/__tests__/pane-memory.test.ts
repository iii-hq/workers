import { describe, expect, it } from 'vitest'
import { createPaneMemory, paneFolderKey } from '../pane-memory'

describe('pane memory', () => {
  it('keeps one value per key, and forgets on request', () => {
    const memory = createPaneMemory<string>()
    memory.write('pane-a', 'query a')
    memory.write('pane-b', 'query b')
    expect(memory.read('pane-a')).toBe('query a')
    memory.forget('pane-a')
    expect(memory.read('pane-a')).toBeUndefined()
    expect(memory.read('pane-b')).toBe('query b')
  })

  it('drops the least recently written key past its limit', () => {
    const memory = createPaneMemory<number>(2)
    memory.write('a', 1)
    memory.write('b', 2)
    memory.write('a', 3)
    memory.write('c', 4)
    expect(memory.read('b')).toBeUndefined()
    expect(memory.read('a')).toBe(3)
    expect(memory.read('c')).toBe(4)
  })

  it('keys folder state by pane and folder', () => {
    expect(paneFolderKey('pane-a', '/repo')).not.toBe(paneFolderKey('pane-a', '/repo-worktree'))
    expect(paneFolderKey('pane-a', '/repo')).not.toBe(paneFolderKey('pane-b', '/repo'))
  })
})
