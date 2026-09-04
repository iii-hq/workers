import { describe, expect, it } from 'vitest'
import { ancestorDirs, basename, breadcrumbSegments, dirname, isUnder, joinRel, validEntryName } from '../paths'
import { duplicateName } from '../file-actions'

describe('paths', () => {
  it('splits names and directories, tolerating the dir slash', () => {
    expect(basename('a/b/c.ts')).toBe('c.ts')
    expect(basename('a/b/')).toBe('b')
    expect(dirname('a/b/c.ts')).toBe('a/b')
    expect(dirname('c.ts')).toBe('')
    expect(joinRel('', 'x')).toBe('x')
    expect(joinRel('a', 'x')).toBe('a/x')
    expect(ancestorDirs('a/b/c.ts')).toEqual(['a', 'a/b'])
    expect(breadcrumbSegments('a/b/c.ts')).toEqual([
      { name: 'a', path: 'a' },
      { name: 'b', path: 'a/b' },
      { name: 'c.ts', path: 'a/b/c.ts' },
    ])
    expect(isUnder('a/b/c.ts', 'a')).toBe(true)
    expect(isUnder('ab/c.ts', 'a')).toBe(false)
    expect(isUnder('x', '')).toBe(true)
  })

  it('validates typed entry names', () => {
    expect(validEntryName('ok.ts')).toBeNull()
    expect(validEntryName('')).not.toBeNull()
    expect(validEntryName('..')).not.toBeNull()
    expect(validEntryName('a/b')).not.toBeNull()
  })

  it('picks the next free duplicate name', () => {
    const taken = new Set(['a/b copy.ts', 'a/b copy 2.ts'])
    expect(duplicateName('a/b.ts', (c) => taken.has(c))).toBe('a/b copy 3.ts')
    expect(duplicateName('.env', () => false)).toBe('.env copy')
  })
})
