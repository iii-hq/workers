import { describe, expect, it } from 'vitest'
import type { FileHit } from './file-search'
import type { FunctionEntry } from './functions'
import {
  MENTION_PAGE_SIZE,
  type MentionCandidate,
  mentionDetail,
  mentionKey,
  mentionName,
  paginateMentions,
  rankMentions,
} from './mention-search'

const functions: FunctionEntry[] = [
  { id: 'shell::exec', description: 'run a command' },
  { id: 'shell::fs::read', description: 'read a file' },
  { id: 'engine::echo', description: 'echo a string back' },
]

const files: FileHit[] = [
  { path: 'shell/src/main.rs', kind: 'file' },
  { path: 'shell/', kind: 'dir' },
  { path: 'console/web/src/App.tsx', kind: 'file' },
]

function names(rows: MentionCandidate[]): string[] {
  return rows.map((row) => (row.kind === 'function' ? row.id : row.path))
}

describe('rankMentions', () => {
  it('keeps catalog order for an empty query: functions, then files', () => {
    expect(names(rankMentions('', functions, files))).toEqual([
      'shell::exec',
      'shell::fs::read',
      'engine::echo',
      'shell/src/main.rs',
      'shell/',
      'console/web/src/App.tsx',
    ])
  })

  it('ranks what starts with the query first and drops non-matches', () => {
    const ranked = names(rankMentions('she', functions, files))
    expect(ranked).toEqual([
      'shell/',
      'shell::exec',
      'shell::fs::read',
      'shell/src/main.rs',
    ])
  })

  it('reads `::` as a function id and a slash as a path', () => {
    expect(names(rankMentions('shell::', functions, files))[0]).toBe(
      'shell::exec',
    )
    expect(names(rankMentions('shell/', functions, files))[0]).toBe('shell/')
  })

  it('matches descriptions and subsequences, after names', () => {
    const ranked = names(rankMentions('echo', functions, files))
    expect(ranked).toEqual(['engine::echo'])
    expect(names(rankMentions('string', functions, files))).toEqual([
      'engine::echo',
    ])
    expect(names(rankMentions('cwsa', functions, files))).toEqual([
      'console/web/src/App.tsx',
    ])
  })

  it('is case-insensitive', () => {
    expect(names(rankMentions('APP', functions, files))).toEqual([
      'console/web/src/App.tsx',
    ])
  })
})

describe('row labels', () => {
  it('splits files into name and folder, functions into id and description', () => {
    const file: MentionCandidate = {
      kind: 'file',
      path: 'console/web/src/App.tsx',
      isDir: false,
    }
    expect(mentionName(file)).toBe('App.tsx')
    expect(mentionDetail(file)).toBe('console/web/src')
    const dir: MentionCandidate = { kind: 'file', path: 'shell/', isDir: true }
    expect(mentionName(dir)).toBe('shell')
    expect(mentionDetail(dir)).toBe('')
    const fn: MentionCandidate = {
      kind: 'function',
      id: 'shell::exec',
      description: 'run',
    }
    expect(mentionName(fn)).toBe('shell::exec')
    expect(mentionDetail(fn)).toBe('run')
    expect(mentionKey(fn)).toBe('fn:shell::exec')
    expect(mentionKey(file)).toBe('file:console/web/src/App.tsx')
  })
})

describe('paginateMentions', () => {
  const items = Array.from({ length: 23 }, (_, i) => i)

  it('shows one page at a time and counts what is left', () => {
    expect(paginateMentions(items, 0)).toEqual({
      visible: items.slice(0, MENTION_PAGE_SIZE),
      remaining: 23 - MENTION_PAGE_SIZE,
    })
    expect(paginateMentions(items, 1).visible).toHaveLength(20)
    expect(paginateMentions(items, 2)).toEqual({ visible: items, remaining: 0 })
    expect(paginateMentions(items, 9)).toEqual({ visible: items, remaining: 0 })
  })
})
