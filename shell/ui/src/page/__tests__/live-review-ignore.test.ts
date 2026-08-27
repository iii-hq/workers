import { describe, expect, it } from 'vitest'
import { onlyIgnoredChanges, trackIgnoredPath } from '../live-review'
import { type ReviewEntry, sameReviewEntry } from '../review'

describe('onlyIgnoredChanges', () => {
  it('is true only when a non-empty burst is entirely ignored', () => {
    const ignored = new Set(['/w/data/a.jsonl', '/w/data/b.json'])
    expect(onlyIgnoredChanges(['/w/data/a.jsonl', '/w/data/b.json'], ignored)).toBe(
      true,
    )
    expect(onlyIgnoredChanges(['/w/data/a.jsonl', '/w/src/main.rs'], ignored)).toBe(
      false,
    )
    expect(onlyIgnoredChanges([], ignored)).toBe(false)
  })
})

describe('trackIgnoredPath', () => {
  it('keeps the latest verdict when a path flips within one burst', () => {
    const ignored = new Set<string>()
    trackIgnoredPath(ignored, '/w/data/a.jsonl', true)
    expect(ignored.has('/w/data/a.jsonl')).toBe(true)
    trackIgnoredPath(ignored, '/w/data/a.jsonl', false)
    expect(ignored.has('/w/data/a.jsonl')).toBe(false)
    trackIgnoredPath(ignored, '/w/src/main.rs', false)
    expect(ignored.size).toBe(0)
  })
})

describe('sameReviewEntry', () => {
  const entry: ReviewEntry = {
    path: 'src/main.rs',
    change: { path: 'src/main.rs', status: 'modified', staged: false },
    baseline: null,
    before: { kind: 'head', path: 'src/main.rs' },
    after: { kind: 'worktree', path: 'src/main.rs' },
  }

  it('is true for an entry git reported identically', () => {
    expect(
      sameReviewEntry({ ...entry, change: { ...entry.change } }, entry),
    ).toBe(true)
  })

  it('is false without a previous entry or when status, staging, or sources move', () => {
    expect(sameReviewEntry(undefined, entry)).toBe(false)
    expect(
      sameReviewEntry(
        { ...entry, change: { ...entry.change, staged: true } },
        entry,
      ),
    ).toBe(false)
    expect(
      sameReviewEntry(
        { ...entry, change: { ...entry.change, status: 'added' } },
        entry,
      ),
    ).toBe(false)
    expect(
      sameReviewEntry({ ...entry, before: { kind: 'empty' } }, entry),
    ).toBe(false)
    expect(
      sameReviewEntry(
        {
          ...entry,
          before: { kind: 'revision', revision: 'abc', path: 'src/main.rs' },
        },
        {
          ...entry,
          before: { kind: 'revision', revision: 'def', path: 'src/main.rs' },
        },
      ),
    ).toBe(false)
  })
})
