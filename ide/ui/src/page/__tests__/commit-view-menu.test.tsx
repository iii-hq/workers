import { copyText } from '@iii-dev/console-ui/format'
import type { ReactElement, ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { CommitView } from '../CommitView'
import type { ContextMenuItem } from '../ContextMenu'
import type { ChangeRow } from '../commit-tree'
import type { GitComparisonEntry } from '../git'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))

// The menus opened, by the test.
const menus = vi.hoisted(() => [] as ContextMenuItem[][])

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = ({ children }: { children?: ReactNode }) => <>{children}</>
  return {
    Button: Pass,
    ConfirmDialog: () => null,
    DropdownMenu: Pass,
    DropdownMenuCheckboxItem: Pass,
    DropdownMenuContent: Pass,
    DropdownMenuLabel: Pass,
    DropdownMenuTrigger: Pass,
    EmptyState: Pass,
    IconButton: Pass,
    Skeleton: () => null,
    StatusPanel: () => null,
    uiClasses: {},
  }
})
vi.mock('@iii-dev/console-ui/format', async (original) => ({
  ...(await original<typeof import('@iii-dev/console-ui/format')>()),
  copyText: vi.fn(async () => false),
}))
vi.mock('../ContextMenu', () => ({
  anchorFromEvent: () => ({ x: 0, y: 0 }),
  useContextMenu: () => ({
    open: (_anchor: unknown, items: ContextMenuItem[]) => menus.push(items),
    element: null,
    isOpen: false,
  }),
}))
vi.mock('../ChangesTree', () => ({ ChangesTree: () => null }))
vi.mock('../CommitBox', () => ({ CommitBox: () => null }))
vi.mock('../RollbackDialog', () => ({ RollbackDialog: () => null }))
vi.mock('../TextDialog', () => ({ TextDialog: () => null }))

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

function entry(path: string, status: GitComparisonEntry['status']): GitComparisonEntry {
  return {
    path,
    status,
    staged: false,
    x: status === 'untracked' ? '?' : ' ',
    y: status === 'untracked' ? '?' : 'M',
    before: { kind: 'head', path },
    after: { kind: 'worktree', path },
  }
}

const modified = entry('src/a.ts', 'modified')
const unversioned = entry('n.md', 'untracked')

type Props = Parameters<typeof CommitView>[0]

function render() {
  const scm = {
    phase: 'ready',
    branch: 'main',
    changes: [modified],
    unversioned: [unversioned],
    included: [],
    isIncluded: () => false,
    setIncluded: vi.fn(),
    error: null,
    refreshing: false,
    busy: false,
    note: null,
    reload: () => {},
    patch: vi.fn(async () => {}),
    run: vi.fn(async () => true),
  }
  const props = {
    host: {},
    root: '/repo',
    scm,
    activePath: null,
    onOpenChange: vi.fn(),
    onOpenFile: vi.fn(),
    onCompare: vi.fn(),
    onShowHistory: vi.fn(),
    onDeleteFile: vi.fn(async () => {}),
  }
  const view = mount(CommitView as unknown as (props: Props) => ReactElement, props as unknown as Props)
  const focus = vi.fn()
  const box = propsWhere(view.result, (p) => 'messageRef' in p) as { messageRef: { current: unknown } }
  box.messageRef.current = { focus }
  /** Right-clicks `target`'s row and picks the item at `path` (a submenu's label first). */
  const choose = (target: GitComparisonEntry, ...path: string[]) => {
    const tree = propsWhere(view.result, (p) => 'onMenu' in p) as { onMenu(row: ChangeRow, anchor: unknown): void }
    const row: ChangeRow = {
      kind: 'file',
      key: target.path,
      group: target.status === 'untracked' ? 'unversioned' : 'changes',
      depth: 1,
      entry: target,
      dir: '',
    }
    tree.onMenu(row, { x: 1, y: 2 })
    let items: readonly ContextMenuItem[] = menus.at(-1) ?? []
    for (const label of path) {
      const item = items.find((candidate) => 'label' in candidate && candidate.label === label)
      if (item?.type === 'submenu') items = item.items
      else if (item && (item.type ?? 'item') === 'item') (item as { onSelect(): void }).onSelect()
      else throw new Error(`no menu item ${label}`)
    }
  }
  const dialog = () =>
    propsWhere(view.result, (p) => p.confirmLabel === 'Delete') as {
      open: boolean
      title: string
      onConfirm(): void
      onCancel(): void
    }
  return { view, scm, props, focus, choose, dialog }
}

describe("the Commit tab's change menu", () => {
  it('commits only the file, from the commit message', () => {
    const { scm, focus, choose } = render()
    choose(modified, 'Commit file…')
    expect(scm.setIncluded.mock.calls).toEqual([
      [[modified, unversioned], false],
      [[modified], true],
    ])
    expect(focus).toHaveBeenCalled()
  })

  it('opens the working copy, compares it and shows its history', () => {
    const { props, choose } = render()
    choose(modified, 'Jump to source')
    expect(props.onOpenFile).toHaveBeenCalledWith('src/a.ts')
    choose(modified, 'Git', 'Compare with…')
    expect(props.onCompare).toHaveBeenCalledWith('src/a.ts')
    choose(modified, 'Git', 'Show history')
    expect(props.onShowHistory).toHaveBeenCalledWith(['src/a.ts'])
  })

  it("deletes a file through the page's delete, only once confirmed", async () => {
    const { scm, props, choose, dialog } = render()
    choose(unversioned, 'Delete…')
    expect(dialog()).toMatchObject({ open: true, title: 'Delete n.md?' })
    dialog().onCancel()
    expect(dialog().open).toBe(false)
    expect(scm.run).not.toHaveBeenCalled()

    choose(unversioned, 'Delete…')
    dialog().onConfirm()
    expect(dialog().open).toBe(false)
    expect(scm.run).toHaveBeenCalledWith('delete', expect.any(Function))
    // The page's delete closes the file's tabs and updates the tree.
    const [, action] = scm.run.mock.calls[0] as unknown as [string, () => Promise<string>]
    expect(await action()).toBe('deleted n.md')
    expect(props.onDeleteFile).toHaveBeenCalledWith('n.md')
    // A refused delete fails the action, so the status line says "delete failed".
    props.onDeleteFile.mockRejectedValueOnce(new Error('permission denied'))
    await expect(action()).rejects.toThrow('permission denied')
  })

  it('says so when the clipboard refuses a copied patch', async () => {
    const { scm, choose } = render()
    choose(modified, 'Copy as patch to clipboard')
    const [entries, use] = scm.patch.mock.calls[0] as unknown as [
      GitComparisonEntry[],
      (patch: string) => Promise<string>,
    ]
    expect(entries).toEqual([modified])
    await expect(use('the patch')).rejects.toThrow('the clipboard refused it')
    expect(copyText).toHaveBeenCalledWith('the patch')
  })
})
