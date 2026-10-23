import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'
import type { GitAction } from '../ActionRail'
import { type FilesView, GitCommitDetails } from '../GitCommitDetails'
import type { CommitDetails, CommitFile } from '../git-log-window'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
  memo: <T>(component: T) => component,
}))

// What the confirmation asked, and the menus opened, by the test.
const seen = vi.hoisted(() => ({ confirms: [] as Array<{ description: string }>, menus: [] as unknown[][] }))

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = () => null
  return {
    Chip: Pass,
    DropdownMenu: Pass,
    DropdownMenuCheckboxItem: Pass,
    DropdownMenuContent: Pass,
    DropdownMenuLabel: Pass,
    DropdownMenuSeparator: Pass,
    DropdownMenuTrigger: Pass,
    EmptyState: Pass,
    IconButton: Pass,
    Skeleton: Pass,
    useConfirm: () => ({
      confirm: async (options: { description: string }) => {
        seen.confirms.push(options)
        return true
      },
      dialog: null,
    }),
  }
})
vi.mock('@iii-dev/console-ui/hooks', () => ({ useSplitDrag: () => ({}) }))
vi.mock('../ActionRail', () => ({ menuItems: (actions: unknown[]) => actions }))
vi.mock('../ContextMenu', () => ({
  useContextMenu: () => ({ open: (_anchor: unknown, items: unknown[]) => seen.menus.push(items), element: null }),
}))
vi.mock('../GitFileList', () => ({ GitFileList: () => null }))
vi.mock('../GitDiffPreview', () => ({ GitDiffPreview: () => null }))
vi.mock('../DiffTab', () => ({ DEFAULT_DIFF_OPTIONS: {} }))

type Props = Parameters<typeof GitCommitDetails>[0]

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

const hasText = (props: Record<string, unknown>, text: string) => [props.children].flat().includes(text)

// c2 renamed index.ts to legacy.ts.
const renamed: CommitFile = {
  path: 'legacy.ts',
  from: 'index.ts',
  status: 'renamed',
  rel: 'legacy.ts',
  view: 'legacy.ts',
}
const details: CommitDetails = {
  sha: 'c2c2c2c2c2',
  parents: ['c1c1c1c1c1'],
  author: 'a',
  authorEmail: 'a@a',
  authorDate: 0,
  committer: 'a',
  committerEmail: 'a@a',
  committerDate: 0,
  message: 'move index.ts',
  files: [renamed],
  truncated: false,
}

function render(view: FilesView, onCommitFiles: Props['onCommitFiles'] = () => {}) {
  const props = {
    host: {},
    root: '/r',
    state: { details, loading: false, error: null, branches: null, signature: null },
    selected: details.sha,
    prefix: '',
    top: '/r',
    shallow: false,
    view,
    onView: () => {},
    busy: false,
    onCommitFiles,
  } as unknown as Props
  return mount(GitCommitDetails as unknown as (props: Props) => ReactElement, props)
}

describe('the commit details', () => {
  it("gets a renamed file from the revision by its own path only, the old name's new file kept", async () => {
    const onCommitFiles = vi.fn()
    const pane = render({ height: null, grouped: true, info: true, preview: false }, onCommitFiles)
    const list = propsWhere(pane.result, (props) => 'onMenu' in props) as {
      onMenu(file: CommitFile, anchor: unknown): void
    }
    list.onMenu(renamed, null)
    const get = (seen.menus.at(-1) as GitAction[]).find((action) => action.id === 'get')
    get?.run()
    await vi.waitFor(() => expect(onCommitFiles).toHaveBeenCalledWith('get', details.sha, ['legacy.ts']))
    expect(seen.confirms.at(-1)?.description).toBe('Uncommitted changes to it are lost.')
    pane.unmount()
  })

  it('keeps the preview its place before a file is picked, so the files do not shrink on the first pick', () => {
    const pane = render({ height: null, grouped: true, info: false, preview: true })
    const root = () => (pane.result as unknown as { props: Record<string, unknown> }).props
    expect(root()['data-info']).toBe(true)
    expect(propsWhere(pane.result, (props) => hasText(props, 'Select a file to preview its diff.'))).not.toBeNull()
    // The preview sits in the details' place: their switch waits for it to go.
    expect(propsWhere(pane.result, (props) => hasText(props, 'Show details'))).toMatchObject({ disabled: true })

    const list = propsWhere(pane.result, (props) => 'onSelect' in props && 'files' in props) as {
      onSelect(file: CommitFile): void
    }
    list.onSelect(renamed)
    expect(root()['data-info']).toBe(true)
    expect(propsWhere(pane.result, (props) => props.file === renamed && 'options' in props)).not.toBeNull()
    pane.unmount()
  })
})
