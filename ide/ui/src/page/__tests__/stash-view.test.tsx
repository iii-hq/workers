import type { ReactElement } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ContextMenuItem } from '../ContextMenu'
import { gitStashApply, gitStashBranch, gitStashClear } from '../git-actions'
import { type GitStash, stashFileSource } from '../git-log'
import { StashView } from '../StashView'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))

// The menus opened, by the test.
const menus = vi.hoisted(() => [] as ContextMenuItem[][])

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = () => null
  return {
    Button: Pass,
    Checkbox: Pass,
    ConfirmDialog: Pass,
    EmptyState: Pass,
    IconButton: Pass,
    Skeleton: Pass,
    StatusPanel: Pass,
    uiClasses: {},
  }
})
vi.mock('../ContextMenu', () => ({
  anchorFromEvent: () => ({ x: 0, y: 0 }),
  useContextMenu: () => ({
    open: (_anchor: unknown, items: ContextMenuItem[]) => menus.push(items),
    element: null,
    isOpen: false,
  }),
}))
vi.mock('../TextDialog', () => ({ TextDialog: () => null }))

const stashes = vi.hoisted<GitStash[]>(() => [
  { ref: 'stash@{0}', sha: 's0', time: 0, branch: 'main', message: 'newer' },
  { ref: 'stash@{1}', sha: 's1', time: 0, branch: 'main', message: 'older' },
])
vi.mock('../git-log', async (original) => ({
  ...(await original<typeof import('../git-log')>()),
  gitStashList: vi.fn(async () => stashes),
  gitStashFiles: vi.fn(async () => [
    { path: 'a.ts', status: 'modified' },
    { path: 'b.ts', status: 'modified' },
  ]),
}))
vi.mock('../git-actions', async (original) => ({
  ...(await original<typeof import('../git-actions')>()),
  gitStashApply: vi.fn(async () => {}),
  gitStashBranch: vi.fn(async () => {}),
  gitStashClear: vi.fn(async () => {}),
  gitStashDrop: vi.fn(async () => {}),
  gitStashPush: vi.fn(async () => {}),
}))

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

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

type Props = Parameters<typeof StashView>[0]
type Dialog = { open: boolean; onConfirm(value?: string): void; onCancel(): void }
type Box = { checked: boolean; onChange(event: unknown): void }

async function render() {
  const host = {} as Props['host']
  const onOpenDiff = vi.fn()
  const props: Props = {
    host,
    root: '/repo',
    refreshEpoch: 0,
    activeSource: null,
    activePath: null,
    onOpenDiff,
    onChanged: () => {},
  }
  const view = mount(StashView as unknown as (props: Props) => ReactElement, props)
  await settle()
  const find = <T,>(test: (props: Record<string, unknown>) => boolean) => propsWhere(view.result, test) as T
  /** Right-clicks the newest stash and picks `label`. */
  const choose = (label: string) => {
    const row = find<{ onContextMenu(event: unknown): void }>((p) => 'onContextMenu' in p)
    const bottom = { getBoundingClientRect: () => ({ bottom: 10 }) }
    row.onContextMenu({ preventDefault: () => {}, currentTarget: { closest: () => bottom } })
    const item = menus.at(-1)?.find((candidate) => 'label' in candidate && candidate.label === label)
    ;(item as { onSelect(): void }).onSelect()
  }
  const clear = () => find<Dialog>((p) => p.title === 'Clear all stashes?')
  const unstash = () => find<Dialog>((p) => p.confirmLabel === 'Unstash')
  const box = (label: string) => find<Box>((p) => p.label === label)
  return { view, host, onOpenDiff, choose, clear, unstash, box }
}

// Calls are counted per test, whatever order the tests run in.
afterEach(() => vi.clearAllMocks())

describe("a stash's menu", () => {
  it('clears every stash, only once confirmed', async () => {
    const { host, choose, clear } = await render()
    choose('Clear…')
    expect(clear().open).toBe(true)
    clear().onCancel()
    expect(clear().open).toBe(false)
    expect(gitStashClear).not.toHaveBeenCalled()

    choose('Clear…')
    clear().onConfirm()
    await settle()
    expect(clear().open).toBe(false)
    expect(gitStashClear).toHaveBeenCalledWith(host, '/repo')
  })

  it('unstashes here with the boxes ticked, or onto a new branch, each time from fresh boxes', async () => {
    const { host, choose, unstash, box } = await render()
    choose('Unstash…')
    expect(unstash().open).toBe(true)
    box('Reinstate index').onChange({ currentTarget: { checked: true } })
    unstash().onConfirm('')
    await settle()
    expect(gitStashApply).toHaveBeenCalledWith(host, '/repo', 'stash@{0}', false, true)

    choose('Unstash…')
    expect(box('Pop stash').checked).toBe(false)
    expect(box('Reinstate index').checked).toBe(false)
    vi.mocked(gitStashApply).mockClear()
    unstash().onConfirm('feat/x')
    await settle()
    expect(gitStashBranch).toHaveBeenCalledWith(host, '/repo', 'feat/x', 'stash@{0}')
    expect(gitStashApply).not.toHaveBeenCalled()
  })

  it("shows the diff of the stash's first file, as a preview", async () => {
    const { onOpenDiff, choose } = await render()
    choose('Show diff')
    await settle()
    expect(onOpenDiff).toHaveBeenCalledWith('a.ts', stashFileSource(stashes[0], {}), false)
  })
})
