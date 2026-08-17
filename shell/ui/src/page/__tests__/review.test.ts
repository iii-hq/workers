import { describe, expect, it } from 'vitest'
import type { GitChange } from '../git'
import {
  diffForReviewEntry,
  mergeGitReviewEntries,
  mergeReviewEntry,
  type ReviewEntry,
  statusForLiveKind,
} from '../review'

describe('statusForLiveKind', () => {
  it('maps watcher events without requiring a repository', () => {
    expect(statusForLiveKind('created')).toBe('untracked')
    expect(statusForLiveKind('modified')).toBe('modified')
    expect(statusForLiveKind('deleted')).toBe('deleted')
  })
})

describe('review entries', () => {
  it('preserves the first baseline across repeated writes', () => {
    const first = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', 'before')
    const second = mergeReviewEntry(first, 'src/app.ts', 'modified', 'after first write')
    expect(second.get('src/app.ts')?.baseline).toBe('before')
  })

  it('marks an unavailable non-Git baseline instead of treating it as an empty file', () => {
    const first = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', undefined)
    const second = mergeReviewEntry(first, 'src/app.ts', 'modified', 'too late')

    expect(second.get('src/app.ts')?.baseline).toBeNull()
  })

  it('fails closed when a modified Git file was not captured before the turn', () => {
    const gitChange: GitChange = { path: 'src/app.ts', status: 'modified', staged: false }

    const entries = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', undefined, gitChange)

    expect(entries.get('src/app.ts')?.baseline).toBeNull()
  })

  it('uses an empty baseline for a new Git file when no earlier body exists', () => {
    const gitChange: GitChange = { path: 'src/new.ts', status: 'untracked', staged: false }

    const entries = mergeReviewEntry(new Map(), 'src/new.ts', 'created', undefined, gitChange)

    expect(entries.get('src/new.ts')?.baseline).toBe('')
  })

  it('gives repeated live writes a fresh diff identity', () => {
    const first = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', 'before')
    const second = mergeReviewEntry(first, 'src/app.ts', 'modified', 'after first write')

    expect(second).not.toBe(first)
    expect(second.get('src/app.ts')?.change).not.toBe(first.get('src/app.ts')?.change)
  })

  it('refreshes identity for repeated writes while Git metadata is unchanged', () => {
    const gitChange: GitChange = { path: 'src/app.ts', status: 'modified', staged: false }
    const first = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', undefined, gitChange)
    const second = mergeReviewEntry(first, 'src/app.ts', 'modified', undefined, gitChange)

    expect(second.get('src/app.ts')?.change).not.toBe(first.get('src/app.ts')?.change)
  })

  it('gives every explicit open a fresh diff identity', () => {
    const entries = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', 'before')
    const entry = entries.get('src/app.ts')!
    const first = diffForReviewEntry(entry)
    const second = diffForReviewEntry(entry)

    expect(second).toEqual(first)
    expect(second.change).not.toBe(first.change)
  })

  it('works for newly created files in an ordinary folder', () => {
    const entries = mergeReviewEntry(new Map(), 'notes/todo.txt', 'created', '')
    expect(entries.get('notes/todo.txt')).toMatchObject({
      baseline: '',
      change: { path: 'notes/todo.txt', status: 'untracked', staged: false },
    })
  })

  it('enriches live rows and seeds existing Git changes', () => {
    const live = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', 'before')
    const changes: GitChange[] = [
      { path: 'src/app.ts', status: 'modified', staged: true },
      { path: 'README.md', status: 'modified', staged: false },
    ]
    const merged = mergeGitReviewEntries(live, changes)
    expect(merged.get('src/app.ts')).toMatchObject({ change: { staged: true } })
    expect(merged.get('src/app.ts')?.baseline).toBe('before')
    expect(merged.get('README.md')?.change).toEqual(changes[1])
  })

  it('can enrich live rows without seeding unrelated Git changes', () => {
    const live = mergeReviewEntry(new Map(), 'src/app.ts', 'modified', 'before')
    const changes: GitChange[] = [
      { path: 'src/app.ts', status: 'modified', staged: true },
      { path: 'README.md', status: 'modified', staged: false },
    ]

    const merged = mergeGitReviewEntries(live, changes, false)

    expect(merged.get('src/app.ts')).toMatchObject({ change: { staged: true } })
    expect(merged.has('README.md')).toBe(false)
  })

  it('reconciles a synthetic source row into a Git rename without seeding other paths', () => {
    const source: ReviewEntry = {
      path: 'src/old.ts',
      change: { path: 'src/old.ts', status: 'deleted', staged: false },
      baseline: 'before',
      gitDir: '/work/repo/src',
    }
    const destination: ReviewEntry = {
      path: 'src/new.ts',
      change: { path: 'src/new.ts', status: 'untracked', staged: false },
      baseline: '',
    }
    const live = new Map([
      [source.path, source],
      [destination.path, destination],
    ])
    const rename: GitChange = {
      path: 'src/new.ts',
      from: 'src/old.ts',
      status: 'renamed',
      staged: false,
    }

    const merged = mergeGitReviewEntries(live, [rename], false)

    expect(merged.has('src/old.ts')).toBe(false)
    expect(merged.get('src/new.ts')?.change).toEqual(rename)
    expect(merged.get('src/new.ts')?.baseline).toBe('before')
    expect(merged.get('src/new.ts')?.gitDir).toBe('/work/repo/src')
  })
})
