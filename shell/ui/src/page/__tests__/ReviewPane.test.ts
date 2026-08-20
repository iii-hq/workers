import { describe, expect, it, vi } from 'vitest'

vi.mock('@iii-dev/console-ui', () => ({
  FileDiff: () => null,
  Markdown: () => null,
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
  Pencil: () => null,
  Save: () => null,
  X: () => null,
}))

import {
  createReviewSaveBarrier,
  defaultCollapsedReviewPaths,
  desiredReviewEntries,
  exactCoderText,
  gitLookupTarget,
  loadReviewContents,
  expandedReviewPaths,
  LARGE_REVIEW_EAGER_FILE_COUNT,
  LARGE_REVIEW_THRESHOLD,
  orderedReviewSummaries,
  prioritizedReviewEntries,
  reportActiveReviewDirty,
  reviewEditStatus,
  runReviewTransition,
  updateReviewDraft,
  reviewEntryWorktreePath,
  withReviewSavePending,
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

describe('inline review editing', () => {
  it('blocks page transitions until a deferred inline save settles', async () => {
    const barrier = createReviewSaveBarrier()
    let finishSave: (() => void) | undefined
    const deferredSave = new Promise<void>((resolve) => {
      finishSave = resolve
    })

    const save = withReviewSavePending(
      'src/app.ts',
      (path, saving) => barrier.update(path, saving),
      () => deferredSave,
    )

    expect([...barrier.paths]).toEqual(['src/app.ts'])
    expect(barrier.canTransition()).toBe(false)
    let waitFinished = false
    const waiting = barrier.wait().then(() => {
      waitFinished = true
    })
    await Promise.resolve()
    expect(waitFinished).toBe(false)

    finishSave?.()
    await save
    await waiting

    expect(barrier.paths.size).toBe(0)
    expect(barrier.canTransition()).toBe(true)
    expect(waitFinished).toBe(true)
  })

  it('blocks editor tab activation and pinning during a deferred inline save', async () => {
    const barrier = createReviewSaveBarrier()
    const activateTab = vi.fn()
    const pinTab = vi.fn()
    let finishSave: (() => void) | undefined
    const save = withReviewSavePending(
      'src/app.ts',
      (path, saving) => barrier.update(path, saving),
      () =>
        new Promise<void>((resolve) => {
          finishSave = resolve
        }),
    )

    expect(runReviewTransition(barrier, activateTab)).toBe(false)
    expect(runReviewTransition(barrier, pinTab)).toBe(false)
    expect(activateTab).not.toHaveBeenCalled()
    expect(pinTab).not.toHaveBeenCalled()

    finishSave?.()
    await save

    expect(runReviewTransition(barrier, activateTab)).toBe(true)
    expect(runReviewTransition(barrier, pinTab)).toBe(true)
    expect(activateTab).toHaveBeenCalledOnce()
    expect(pinTab).toHaveBeenCalledOnce()
  })

  it('allows a paused working-directory transition to retry after a save fails', async () => {
    const barrier = createReviewSaveBarrier()
    const changeRoot = vi.fn()
    let failSave: ((error: Error) => void) | undefined
    const save = withReviewSavePending(
      'src/app.ts',
      (path, saving) => barrier.update(path, saving),
      () =>
        new Promise<void>((_resolve, reject) => {
          failSave = reject
        }),
    )

    expect(runReviewTransition(barrier, changeRoot)).toBe(false)
    expect(changeRoot).not.toHaveBeenCalled()

    failSave?.(new Error('save failed'))
    await expect(save).rejects.toThrow('save failed')

    expect(runReviewTransition(barrier, changeRoot)).toBe(true)
    expect(changeRoot).toHaveBeenCalledOnce()
  })

  it('rejects editor changes synchronously while a save is pending', () => {
    const setDraft = vi.fn()

    updateReviewDraft({ current: true }, 'post-submit edit', setDraft)
    expect(setDraft).not.toHaveBeenCalled()

    updateReviewDraft({ current: false }, 'accepted edit', setDraft)
    expect(setDraft).toHaveBeenCalledOnce()
    expect(setDraft).toHaveBeenCalledWith('accepted edit')
  })

  it('does not clear page-owned dirty state from an inactive row', () => {
    const onChange = vi.fn()

    reportActiveReviewDirty(false, 'src/app.ts', false, onChange)
    expect(onChange).not.toHaveBeenCalled()

    reportActiveReviewDirty(true, 'src/app.ts', true, onChange)
    expect(onChange).toHaveBeenCalledWith('src/app.ts', true)
  })

  it('describes editor loading, failure, and save progress visibly', () => {
    expect(reviewEditStatus(false, 'loading', false)).toBeNull()
    expect(reviewEditStatus(true, 'loading', false)).toEqual({
      role: 'status',
      message: 'loading inline editor…',
    })
    expect(reviewEditStatus(true, 'error', false)).toEqual({
      role: 'alert',
      message: 'inline editor failed to load; cancel and try again',
    })
    expect(reviewEditStatus(true, 'ready', true)).toEqual({
      role: 'status',
      message: 'saving file…',
    })
    expect(reviewEditStatus(true, 'ready', false)).toBeNull()
  })
})

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

describe('reviewEntryWorktreePath', () => {
  it('allows live and worktree-backed comparisons to edit the current file', () => {
    expect(reviewEntryWorktreePath(entry('src/live.ts'))).toBe('src/live.ts')
    expect(
      reviewEntryWorktreePath({
        ...entry('src/unstaged.ts'),
        before: { kind: 'index', path: 'src/unstaged.ts' },
        after: { kind: 'worktree', path: 'src/unstaged.ts' },
      }),
    ).toBe('src/unstaged.ts')
  })

  it('keeps staged, revision, and deleted targets read-only', () => {
    expect(
      reviewEntryWorktreePath({
        ...entry('src/staged.ts'),
        before: { kind: 'head', path: 'src/staged.ts' },
        after: { kind: 'index', path: 'src/staged.ts' },
      }),
    ).toBeNull()
    expect(
      reviewEntryWorktreePath({
        ...entry('src/committed.ts'),
        before: { kind: 'revision', revision: 'a', path: 'src/committed.ts' },
        after: { kind: 'revision', revision: 'b', path: 'src/committed.ts' },
      }),
    ).toBeNull()
    expect(
      reviewEntryWorktreePath({
        ...entry('src/deleted.ts'),
        change: {
          path: 'src/deleted.ts',
          status: 'deleted',
          staged: false,
        },
      }),
    ).toBeNull()
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

describe('gitLookupTarget', () => {
  it('runs Git in the file own directory so a nested repository answers', () => {
    expect(gitLookupTarget('/root', 'nested/repo/src/app.ts')).toEqual({
      cwd: '/root/nested/repo/src',
      name: 'app.ts',
    })
    expect(gitLookupTarget('/root', 'top.ts')).toEqual({
      cwd: '/root',
      name: 'top.ts',
    })
  })
})

function execHost(replies: Record<string, unknown>) {
  const trigger = vi.fn(async (functionId: string, input: unknown) => {
    const reply = replies[functionId]
    if (reply === undefined) throw new Error(`unexpected function ${functionId}`)
    return typeof reply === 'function'
      ? (reply as (value: unknown) => unknown)(input)
      : reply
  })
  return {
    host: { iii: { trigger } } as unknown as Parameters<typeof loadReviewContents>[0],
    trigger,
  }
}

describe('loadReviewContents without a captured baseline', () => {
  const uncaptured: ReviewEntry = {
    path: 'nested/repo/src/app.ts',
    change: { path: 'nested/repo/src/app.ts', status: 'modified', staged: false },
    baseline: null,
  }

  it('falls back to the committed body and labels it', async () => {
    const { host, trigger } = execHost({
      'shell::exec': {
        exit_code: 0,
        stdout: 'committed\n',
        stderr: '',
        timed_out: false,
        stdout_truncated: false,
        stderr_truncated: false,
      },
      'coder::read-file': {
        content: 'current\n',
        is_utf8: true,
        more_lines: false,
        revision: 'r1',
        mode: 420,
      },
    })

    await expect(loadReviewContents(host, '/root', uncaptured)).resolves.toEqual({
      oldContents: 'committed\n',
      newContents: 'current\n',
      worktreeRevision: 'r1',
      mode: 420,
      baselineSource: 'committed',
    })
    expect(trigger).toHaveBeenCalledWith(
      'shell::exec',
      expect.objectContaining({ cwd: '/root/nested/repo/src' }),
    )
  })

  it('keeps failing closed when there is no committed body either', async () => {
    const { host } = execHost({
      'shell::exec': {
        exit_code: 128,
        stdout: '',
        stderr: 'fatal: not a git repository',
        timed_out: false,
        stdout_truncated: false,
        stderr_truncated: false,
      },
    })

    await expect(loadReviewContents(host, '/root', uncaptured)).rejects.toThrow(
      'earlier content was not captured for this turn',
    )
  })

  it('compares a deleted file against its committed body', async () => {
    const { host } = execHost({
      'shell::exec': {
        exit_code: 0,
        stdout: 'committed\n',
        stderr: '',
        timed_out: false,
        stdout_truncated: false,
        stderr_truncated: false,
      },
    })

    await expect(
      loadReviewContents(host, '/root', {
        ...uncaptured,
        change: { ...uncaptured.change, status: 'deleted' },
      }),
    ).resolves.toEqual({
      oldContents: 'committed\n',
      newContents: '',
      baselineSource: 'committed',
    })
  })
})

describe('loadReviewContents for raster images', () => {
  it('loads the working copy as a data URL instead of a text diff', async () => {
    const { host, trigger } = execHost({
      'coder::read-file': { content: 'iVBORw0KGgo=', revision: 'r9', mode: 420, size: 8 },
    })
    const image: ReviewEntry = {
      path: 'docs/logo.png',
      change: { path: 'docs/logo.png', status: 'modified', staged: false },
      baseline: 'old bytes',
    }

    await expect(loadReviewContents(host, '/root', image)).resolves.toEqual({
      oldContents: '',
      newContents: '',
      worktreeRevision: 'r9',
      mode: 420,
      image: 'data:image/png;base64,iVBORw0KGgo=',
    })
    expect(trigger).toHaveBeenCalledWith(
      'coder::read-file',
      expect.objectContaining({ path: '/root/docs/logo.png', encoding: 'base64' }),
    )
    expect(trigger).toHaveBeenCalledTimes(1)
  })

  it('flags a committed image as unavailable instead of deleted', async () => {
    const { host, trigger } = execHost({})
    const committed: ReviewEntry = {
      path: 'docs/logo.png',
      change: { path: 'docs/logo.png', status: 'modified', staged: false },
      before: { kind: 'revision', revision: 'aaaa', path: 'docs/logo.png' },
      after: { kind: 'revision', revision: 'bbbb', path: 'docs/logo.png' },
    }

    await expect(loadReviewContents(host, '/root', committed)).resolves.toEqual({
      oldContents: '',
      newContents: '',
      imageUnavailable: true,
    })
    expect(trigger).not.toHaveBeenCalled()
  })

  it('marks a deleted image without reading anything', async () => {
    const { host, trigger } = execHost({})
    const deleted: ReviewEntry = {
      path: 'docs/logo.png',
      change: { path: 'docs/logo.png', status: 'deleted', staged: false },
      baseline: 'old bytes',
    }

    await expect(loadReviewContents(host, '/root', deleted)).resolves.toEqual({
      oldContents: '',
      newContents: '',
      image: null,
    })
    expect(trigger).not.toHaveBeenCalled()
  })
})
