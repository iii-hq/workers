import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  __resetFileSearchCacheForTests,
  createWorkspaceFileSearch,
  FILE_SEARCH_LIMIT,
  hitsFromSearchOutput,
  isHiddenPath,
  relativeToWorkingDir,
  SEARCH_FUNCTION_ID,
  searchWorkspaceFiles,
} from './file-search'

describe('hitsFromSearchOutput', () => {
  it('relativizes, marks folders, and drops hidden or paren paths', () => {
    expect(
      hitsFromSearchOutput('/w', {
        path_matches: [
          { path: '/w/src/main.rs', kind: 'file' },
          { path: '/w/src', kind: 'dir' },
          { path: '/w/.github/ci.yml', kind: 'file' },
          { path: '/w/notes (copy).md', kind: 'file' },
          { path: '/w', kind: 'dir' },
          { path: '/elsewhere/x.ts', kind: 'file' },
        ],
      }),
    ).toEqual([
      { path: 'src/main.rs', kind: 'file' },
      { path: 'src/', kind: 'dir' },
      { path: '/elsewhere/x.ts', kind: 'file' },
    ])
  })

  it('tolerates a missing match list', () => {
    expect(hitsFromSearchOutput('/w', {})).toEqual([])
  })
})

describe('path helpers', () => {
  it('relativizes under the working dir only', () => {
    expect(relativeToWorkingDir('/w', '/w/a/b.ts')).toBe('a/b.ts')
    expect(relativeToWorkingDir('/w/', '/w/a/b.ts')).toBe('a/b.ts')
    expect(relativeToWorkingDir('/w', '/w')).toBe('')
    expect(relativeToWorkingDir('/w', '/w-other/a.ts')).toBe('/w-other/a.ts')
  })

  it('flags dot segments anywhere in the path', () => {
    expect(isHiddenPath('.env')).toBe(true)
    expect(isHiddenPath('src/.cache/x')).toBe(true)
    expect(isHiddenPath('src/a.b.ts')).toBe(false)
  })
})

describe('searchWorkspaceFiles', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    __resetFileSearchCacheForTests()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('asks the worker for a quick-open search scoped to the working dir', async () => {
    const trigger = vi.fn().mockResolvedValue({
      path_matches: [{ path: '/w/src/a.ts', kind: 'file' }],
    })
    const hits = await searchWorkspaceFiles('/w', ' a ', {}, trigger)
    expect(trigger).toHaveBeenCalledWith(SEARCH_FUNCTION_ID, {
      query: 'a',
      path: '.',
      fs_scope: { root: '/w', boundary: 'workspace' },
      ignore_case: true,
      search_content: false,
      search_paths: true,
      fuzzy_paths: true,
      respect_gitignore: true,
      include_hidden: false,
      use_default_excludes: true,
      max_matches: FILE_SEARCH_LIMIT,
    })
    expect(hits).toEqual([{ path: 'src/a.ts', kind: 'file' }])
  })

  it('reuses a fresh result and shares an in-flight request', async () => {
    const trigger = vi.fn().mockResolvedValue({ path_matches: [] })
    const first = searchWorkspaceFiles('/w', 'q', {}, trigger)
    const second = searchWorkspaceFiles('/w', 'q', {}, trigger)
    await Promise.all([first, second])
    await searchWorkspaceFiles('/w', 'q', {}, trigger)
    expect(trigger).toHaveBeenCalledTimes(1)
    vi.advanceTimersByTime(20_000)
    await searchWorkspaceFiles('/w', 'q', {}, trigger)
    expect(trigger).toHaveBeenCalledTimes(2)
  })

  it('resolves to no hits when the worker fails', async () => {
    const trigger = vi.fn().mockRejectedValue(new Error('away'))
    await expect(searchWorkspaceFiles('/w', 'q', {}, trigger)).resolves.toEqual(
      [],
    )
  })

  it('binds a working dir for the composer', async () => {
    const search = createWorkspaceFileSearch('/w')
    expect(typeof search).toBe('function')
  })
})
