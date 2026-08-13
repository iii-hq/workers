import { describe, expect, it, vi } from 'vitest'

vi.mock('@iii-dev/console-ui', () => ({
  FileDiff: () => null,
}))
vi.mock('react', () => ({
  useCallback: (callback: unknown) => callback,
  useEffect: () => undefined,
  useMemo: (factory: () => unknown) => factory(),
  useRef: (value: unknown) => ({ current: value }),
  useState: (value: unknown) => [
    typeof value === 'function' ? (value as () => unknown)() : value,
    () => undefined,
  ],
}))
vi.mock('react/jsx-runtime', () => ({
  Fragment: Symbol('Fragment'),
  jsx: () => null,
  jsxs: () => null,
}))
vi.mock('react/jsx-dev-runtime', () => ({
  Fragment: Symbol('Fragment'),
  jsxDEV: () => null,
}))
vi.mock('lucide-react', () => ({
  ChevronDown: () => null,
  ChevronRight: () => null,
  FileCode2: () => null,
}))

import {
  defaultCollapsedReviewPaths,
  desiredReviewEntries,
  exactCoderText,
  expandedReviewPaths,
  LARGE_REVIEW_EAGER_FILE_COUNT,
  LARGE_REVIEW_THRESHOLD,
  orderedReviewSummaries,
  prioritizedReviewEntries,
} from '../ReviewPane'
import type { ReviewEntry } from '../review'

function entry(path: string): ReviewEntry {
  return {
    path,
    change: {
      path,
      status: 'modified',
      staged: false,
    },
    baseline: '',
  }
}

describe('large ReviewPane scheduling', () => {
  it('keeps small reviews fully expanded', () => {
    const entries = Array.from(
      { length: LARGE_REVIEW_THRESHOLD },
      (_, index) => entry(`src/file-${String(index)}.ts`),
    )

    expect(defaultCollapsedReviewPaths(entries).size).toBe(0)
  })

  it('caps the initially expanded files in broad reviews', () => {
    const entries = Array.from(
      { length: LARGE_REVIEW_THRESHOLD + 5 },
      (_, index) => entry(`src/file-${String(index)}.ts`),
    )

    const collapsed = defaultCollapsedReviewPaths(entries)
    expect(collapsed.size).toBe(entries.length - LARGE_REVIEW_EAGER_FILE_COUNT)
    expect(collapsed.has(entries[LARGE_REVIEW_EAGER_FILE_COUNT - 1].path)).toBe(false)
    expect(collapsed.has(entries[LARGE_REVIEW_EAGER_FILE_COUNT].path)).toBe(true)
  })

  it('loads the active file first without disturbing remaining order', () => {
    const entries = [entry('a.ts'), entry('b.ts'), entry('c.ts')]

    expect(prioritizedReviewEntries(entries, 'c.ts').map((item) => item.path)).toEqual([
      'c.ts',
      'a.ts',
      'b.ts',
    ])
    expect(prioritizedReviewEntries(entries, 'missing.ts')).toBe(entries)
  })

  it('loads only active, expanded eager, and viewport-requested files in a broad review', () => {
    const entries = Array.from(
      { length: LARGE_REVIEW_THRESHOLD + 5 },
      (_, index) => entry(`src/file-${String(index)}.ts`),
    )
    const collapsed = defaultCollapsedReviewPaths(entries)
    collapsed.add(entries[1].path)
    const viewportRequested = new Set([
      entries[LARGE_REVIEW_EAGER_FILE_COUNT + 1].path,
      entries[LARGE_REVIEW_EAGER_FILE_COUNT + 2].path,
    ])
    collapsed.delete(entries[LARGE_REVIEW_EAGER_FILE_COUNT + 1].path)

    expect(
      desiredReviewEntries(
        entries,
        entries.at(-1)?.path ?? null,
        collapsed,
        viewportRequested,
      ).map((item) => item.path),
    ).toEqual([
      entries.at(-1)?.path,
      ...entries
        .slice(0, LARGE_REVIEW_EAGER_FILE_COUNT)
        .filter((item) => item.path !== entries[1].path)
        .map((item) => item.path),
      entries[LARGE_REVIEW_EAGER_FILE_COUNT + 1].path,
    ])
  })

  it('does not hydrate every file when a broad review is expanded', () => {
    const entries = Array.from(
      { length: LARGE_REVIEW_THRESHOLD + 5 },
      (_, index) => entry(`src/file-${String(index)}.ts`),
    )

    expect(
      desiredReviewEntries(entries, null, new Set(), new Set()).map(
        (item) => item.path,
      ),
    ).toEqual(
      entries.slice(0, LARGE_REVIEW_EAGER_FILE_COUNT).map((item) => item.path),
    )
  })

  it('preserves eager hydration for small reviews', () => {
    const entries = [entry('a.ts'), entry('b.ts'), entry('c.ts')]

    expect(
      desiredReviewEntries(
        entries,
        'c.ts',
        new Set(entries.map((item) => item.path)),
        new Set(),
      ).map((item) => item.path),
    ).toEqual(['c.ts', 'a.ts', 'b.ts'])
  })

  it('re-expands an active file on repeat activation', () => {
    const collapsed = new Set(['active.ts', 'other.ts'])
    const first = expandedReviewPaths(collapsed, 'active.ts')
    const repeated = expandedReviewPaths(first, 'active.ts')

    expect([...first]).toEqual(['other.ts'])
    expect(repeated).toBe(first)
  })
})

describe('exactCoderText', () => {
  it('rejects partial reads instead of diffing incomplete content', () => {
    expect(() =>
      exactCoderText(
        { content: 'partial\n', is_utf8: true, more_lines: true },
        'src/large.ts',
      ),
    ).toThrow('file read was truncated: src/large.ts')
  })

  it('accepts a complete UTF-8 read', () => {
    expect(
      exactCoderText(
        { content: 'complete\n', is_utf8: true, more_lines: false },
        'src/app.ts',
      ),
    ).toBe('complete\n')
  })
})

describe('orderedReviewSummaries', () => {
  it('keeps every path visible without claiming unloaded files have zero changes', () => {
    const entries = [entry('loaded.ts'), entry('waiting.ts'), entry('failed.ts')]
    const loaded = {
      path: 'loaded.ts',
      state: 'ready' as const,
      add: 4,
      del: 2,
      oldContents: 'old',
      newContents: 'new',
    }

    expect(
      orderedReviewSummaries(
        entries,
        new Map([['loaded.ts', loaded]]),
        new Set(['loaded.ts']),
        new Set(['failed.ts']),
      ),
    ).toEqual([
      loaded,
      {
        path: 'waiting.ts',
        state: 'pending',
        add: null,
        del: null,
        oldContents: null,
        newContents: null,
      },
      {
        path: 'failed.ts',
        state: 'unavailable',
        add: null,
        del: null,
        oldContents: null,
        newContents: null,
      },
    ])
  })

  it('does not reuse stale totals while a refreshed file reloads', () => {
    const entries = [entry('refreshing.ts')]
    const previous = {
      path: 'refreshing.ts',
      state: 'ready' as const,
      add: 9,
      del: 1,
      oldContents: 'old',
      newContents: 'new',
    }

    expect(
      orderedReviewSummaries(entries, new Map([['refreshing.ts', previous]]), new Set(), new Set()),
    ).toEqual([
      {
        path: 'refreshing.ts',
        state: 'pending',
        add: null,
        del: null,
        oldContents: null,
        newContents: null,
      },
    ])
  })
})
