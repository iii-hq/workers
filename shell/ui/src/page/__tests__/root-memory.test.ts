import { describe, expect, it } from 'vitest'
import {
  EMPTY_ROOT_MEMORY,
  MAX_REMEMBERED_ROOTS,
  parseRootMemory,
  recallRoot,
  rememberRoot,
  serializeRootMemory,
} from '../root-memory'

const state = (path: string) => ({
  open: [{ kind: 'file', path, pinned: true }],
  active: `file:${path}`,
  expanded: ['src'],
})

describe('root-memory', () => {
  it('remembers a folder and recalls it', () => {
    const memory = rememberRoot(EMPTY_ROOT_MEMORY, '/a', state('a.ts'))
    expect(recallRoot(memory, '/a')).toEqual(state('a.ts'))
    expect(recallRoot(memory, '/b')).toBeNull()
    expect(EMPTY_ROOT_MEMORY.size).toBe(0)
  })

  it('forgets a folder with nothing open or expanded', () => {
    const memory = rememberRoot(rememberRoot(EMPTY_ROOT_MEMORY, '/a', state('a.ts')), '/a', {
      open: [],
      active: null,
      expanded: [],
    })
    expect(recallRoot(memory, '/a')).toBeNull()
  })

  it('keeps the most recent folders and drops the oldest', () => {
    let memory = EMPTY_ROOT_MEMORY
    for (let index = 0; index < MAX_REMEMBERED_ROOTS + 3; index += 1) {
      memory = rememberRoot(memory, `/root-${index}`, state(`${index}.ts`))
    }
    expect(memory.size).toBe(MAX_REMEMBERED_ROOTS)
    expect(recallRoot(memory, '/root-0')).toBeNull()
    expect(recallRoot(memory, '/root-2')).toBeNull()
    expect(recallRoot(memory, '/root-3')).not.toBeNull()
    // Touching an old folder moves it to the front.
    memory = rememberRoot(memory, '/root-3', state('again.ts'))
    memory = rememberRoot(memory, '/fresh', state('fresh.ts'))
    expect(recallRoot(memory, '/root-3')?.active).toBe('file:again.ts')
    expect(recallRoot(memory, '/root-4')).toBeNull()
  })

  it('round-trips through the persisted object and drops junk', () => {
    const memory = rememberRoot(rememberRoot(EMPTY_ROOT_MEMORY, '/a', state('a.ts')), '/b', state('b.ts'))
    const parsed = parseRootMemory(serializeRootMemory(memory))
    expect([...parsed.keys()]).toEqual(['/a', '/b'])
    expect(recallRoot(parsed, '/b')).toEqual(state('b.ts'))
    const junk = parseRootMemory({
      '': state('x'),
      '/junk': 'nope',
      '/half': { open: 'not-a-list', active: 3, expanded: ['ok', 7] },
      '/empty': { open: [], active: null, expanded: [] },
    })
    expect([...junk.keys()]).toEqual(['/half'])
    expect(recallRoot(junk, '/half')).toEqual({ open: [], active: null, expanded: ['ok'] })
    expect(parseRootMemory(null).size).toBe(0)
    expect(parseRootMemory([1, 2]).size).toBe(0)
  })
})
