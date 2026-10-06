import { describe, expect, it, vi } from 'vitest'
import { tickState } from '../ChangesTree'
import { changeRows, changeSummary, defaultOpen, entryPaths, expandableKeys } from '../commit-tree'
import type { GitComparisonEntry } from '../git'
import { matchesCommit } from '../HistoryView'
import { buildChangeTree } from '../scm-view'
import { isNoModelError, modelLabel } from '../use-commit-message'

// The shared components only exist inside the console; these tests read pure helpers.
vi.mock('@iii-dev/console-ui', () => ({}))
// Counted, to see when a group's tree is built again.
vi.mock('../scm-view', async (original) => {
  const actual = await original<typeof import('../scm-view')>()
  return { ...actual, buildChangeTree: vi.fn(actual.buildChangeTree) }
})

function entry(
  path: string,
  status: GitComparisonEntry['status'] = 'modified',
  renameFrom?: string,
): GitComparisonEntry {
  return {
    path,
    status,
    staged: false,
    x: status === 'untracked' ? '?' : ' ',
    y: status === 'untracked' ? '?' : 'M',
    before: { kind: 'head', path },
    after: { kind: 'worktree', path },
    ...(renameFrom ? { renameFrom } : {}),
  }
}

const everythingOpen = { byDirectory: true, isOpen: (_key: string, fallback: boolean) => fallback }

describe('changeRows', () => {
  it('folds single-child folder chains and keeps unversioned folders closed by default', () => {
    const rows = changeRows(
      [
        {
          id: 'changes',
          label: 'Changes',
          entries: [entry('tech-specs/2026-06-agentic/llm-router.md'), entry('README.md')],
        },
        {
          id: 'unversioned',
          label: 'Unversioned files',
          entries: [entry('console/web/a.ts', 'untracked'), entry('console/web/b.ts', 'untracked')],
        },
      ],
      everythingOpen,
    )
    expect(rows.map((row) => [row.kind, row.kind === 'file' ? row.entry.path : row.label, row.depth])).toEqual([
      ['group', 'Changes', 0],
      ['folder', 'tech-specs/2026-06-agentic', 1],
      ['file', 'tech-specs/2026-06-agentic/llm-router.md', 2],
      ['file', 'README.md', 1],
      ['group', 'Unversioned files', 0],
      ['folder', 'console/web', 1],
    ])
    const folder = rows.find((row) => row.kind === 'folder' && row.group === 'unversioned')
    expect(folder?.kind === 'folder' && folder.entries.map((e) => e.path)).toEqual([
      'console/web/a.ts',
      'console/web/b.ts',
    ])
  })

  it('lists files flat with their folder when not grouping by directory, and skips empty or closed groups', () => {
    const rows = changeRows(
      [
        { id: 'changes', label: 'Changes', entries: [entry('src/a.ts')] },
        { id: 'unversioned', label: 'Unversioned files', entries: [] },
      ],
      { byDirectory: false, isOpen: (_key, fallback) => fallback },
    )
    expect(rows).toHaveLength(2)
    expect(rows[1]).toMatchObject({ kind: 'file', dir: 'src', depth: 1 })
    const closed = changeRows([{ id: 'changes', label: 'Changes', entries: [entry('src/a.ts')] }], {
      byDirectory: true,
      isOpen: () => false,
    })
    expect(closed.map((row) => row.kind)).toEqual(['group'])
  })

  it('names every expandable key for expand and collapse all', () => {
    const groups = [{ id: 'changes' as const, label: 'Changes', entries: [entry('a/b/c.ts'), entry('a/d.ts')] }]
    expect(expandableKeys(groups)).toEqual(['changes', 'changes:a/', 'changes:a/b/'])
    expect(defaultOpen('folder', 'unversioned')).toBe(false)
    expect(defaultOpen('group', 'unversioned')).toBe(true)
  })

  it("builds a group's tree once per entries array: toggles and Expand all reuse it", () => {
    const entries = [entry('a/b/c.ts'), entry('a/d.ts')]
    const groups = [{ id: 'changes' as const, label: 'Changes', entries }]
    vi.mocked(buildChangeTree).mockClear()
    changeRows(groups, everythingOpen)
    changeRows(groups, { byDirectory: true, isOpen: () => false })
    expandableKeys(groups)
    expect(buildChangeTree).toHaveBeenCalledTimes(1)
    // A new status is a new array, and builds again.
    expect(changeRows([{ ...groups[0], entries: [...entries, entry('a/e.ts')] }], everythingOpen)).toContainEqual(
      expect.objectContaining({ key: 'changes:a/e.ts' }),
    )
    expect(buildChangeTree).toHaveBeenCalledTimes(2)
  })
})

describe('summaries', () => {
  it('counts by kind in a fixed order', () => {
    expect(changeSummary([entry('a', 'untracked'), entry('b'), entry('c', 'added'), entry('d')])).toBe(
      '2 modified, 1 added, 1 unversioned',
    )
    expect(changeSummary([])).toBe('')
  })

  it('names both ends of a rename', () => {
    expect(entryPaths([entry('new.ts', 'renamed', 'old.ts'), entry('a.ts')])).toEqual(['old.ts', 'new.ts', 'a.ts'])
  })

  it('ticks a folder fully, partly or not at all', () => {
    const entries = [entry('a'), entry('b')]
    expect(tickState(entries, () => true)).toBe('on')
    expect(tickState(entries, (e) => e.path === 'a')).toBe('mixed')
    expect(tickState(entries, () => false)).toBe('off')
  })
})

describe('Generate and History helpers', () => {
  it('shows a model without its provider and recognizes the no-model error', () => {
    expect(modelLabel('anthropic::claude-haiku-4-5')).toBe('claude-haiku-4-5')
    expect(modelLabel('gpt-5')).toBe('gpt-5')
    expect(isNoModelError('NO_MODEL', 'x')).toBe(true)
    expect(isNoModelError(undefined, 'no model for commit messages: …')).toBe(true)
    expect(isNoModelError('ROUTER_FAILED', 'router::complete failed')).toBe(false)
  })

  it('filters commits by subject, author or hash', () => {
    const commit = { subject: 'fix(ide): keep ticks', author: 'Ana Lima', sha: 'abc123def' }
    expect(matchesCommit(commit, '')).toBe(true)
    expect(matchesCommit(commit, 'TICKS')).toBe(true)
    expect(matchesCommit(commit, 'lima')).toBe(true)
    expect(matchesCommit(commit, 'abc12')).toBe(true)
    expect(matchesCommit(commit, 'nope')).toBe(false)
  })
})
