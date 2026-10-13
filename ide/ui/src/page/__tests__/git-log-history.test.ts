import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { GitLogTab } from '../GitLogTab'
import { GitToolWindow } from '../GitToolWindow'
import { type LogFilter, logArgs, type RefsSnapshot } from '../git-log-window'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
  memo: <T>(component: T) => component,
}))

// The filter each render handed the log.
const filters = vi.hoisted(() => [] as LogFilter[])

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = () => null
  return {
    ConfirmDialog: Pass,
    EmptyState: Pass,
    LiveRegion: Pass,
    Tabs: Pass,
    TabsContent: Pass,
    TabsList: Pass,
    TabsTrigger: Pass,
    Toolbar: Pass,
    Tooltip: Pass,
  }
})
vi.mock('@iii-dev/console-ui/hooks', () => ({
  useDebounce: <T>(value: T) => value,
  usePaneState: <T>(_key: string, initial: T) => [initial, () => {}],
  useSplitDrag: () => ({}),
}))
vi.mock('../ContextMenu', async (original) => ({
  ...(await original<typeof import('../ContextMenu')>()),
  useContextMenu: () => ({ open: () => {}, element: null, isOpen: false }),
}))
vi.mock('../use-worktree-ops', async (original) => ({
  ...(await original<typeof import('../use-worktree-ops')>()),
  useWorktreeEpoch: () => 0,
  useWorktreeOps: () => ({ list: null, noteSeq: 0, note: null, noteIsMine: false, busy: false }),
}))
// The IDE's folder is `ide/` in its repository.
const snapshot: RefsSnapshot = { refs: [], head: 'h', shallow: false, prefix: 'ide/', truncated: false, signature: 's' }
vi.mock('../use-git-log', async (original) => ({
  ...(await original<typeof import('../use-git-log')>()),
  useGitLog: (_host: unknown, _root: string, _epoch: number, _active: boolean, filter: LogFilter) => {
    filters.push(filter)
    return { snapshot, notRepo: false, commits: [], graph: null, done: true, loading: false, error: null }
  },
  useCommitDetails: () => ({ details: null, loading: false, error: null }),
  useWorkingDiff: () => ({ files: [], loading: false, error: null }),
}))

type WindowProps = Parameters<typeof GitToolWindow>[0]
type LogProps = Parameters<typeof GitLogTab>[0]

/** The props of the first element in `node` that `test` accepts. */
function propsWhere(node: unknown, test: (props: Record<string, unknown>) => boolean): Record<string, unknown> | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const hit = propsWhere(child, test)
      if (hit) return hit
    }
    return null
  }
  if (node === null || typeof node !== 'object') return null
  const props = (node as { props?: Record<string, unknown> }).props
  if (props === undefined) return null
  return test(props) ? props : propsWhere(props.children, test)
}

describe('Show history', () => {
  it('narrows the git log to the file, and a second one to the next file, its whole history', () => {
    const open = (path: string, seq: number) =>
      ({
        host: {},
        root: '/repo/ide',
        page: {},
        tab: 'log',
        onTabChange: () => {},
        onHide: () => {},
        paneKey: 'p',
        onOpenCommitFile: () => {},
        onOpenCompareFile: () => {},
        onOpenWorkingFile: () => {},
        onOpenRevision: () => {},
        focusPaths: { paths: [path], seq },
      }) as unknown as WindowProps
    const gitWindow = mount(GitToolWindow as unknown as (props: WindowProps) => ReactElement, open('src/a.ts', 1))
    const logTab = () => propsWhere(gitWindow.result, (props) => 'focusPaths' in props) as unknown as LogProps
    const log = mount(GitLogTab as (props: LogProps) => ReactElement, logTab())
    // The pathspec ends the command: the file, from the repository's top.
    const pathspec = () => logArgs(filters.at(-1) ?? {}, 0, 1).slice(-2)
    expect(pathspec()).toEqual(['--', ':(top,literal)ide/src/a.ts'])
    // A "History up to here" from the commit details cuts the log at a commit.
    const details = propsWhere(log.result, (props) => 'onHistory' in props) as {
      onHistory(paths: string[], sha: string): void
    }
    details.onHistory(['ide/src/a.ts'], 'abc1234')
    expect(filters.at(-1)?.upTo).toBe('abc1234')

    gitWindow.rerender(open('b.md', 2))
    log.rerender(logTab())
    expect(pathspec()).toEqual(['--', ':(top,literal)ide/b.md'])
    // The next Show history reads the file's whole history.
    expect(filters.at(-1)?.upTo).toBeUndefined()
  })
})
