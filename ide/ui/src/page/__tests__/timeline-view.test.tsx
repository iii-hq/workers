import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TimelineTab } from '../TimelineTab'
import { buildChangeTree, readScmViewMode, relativeDisplayPath, writeScmViewMode } from '../scm-view'
import { relativeToRoot, type TurnFileHead } from '../turns'

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({
  IconButton: ({ label, children, ...rest }: { label: string; children: ReactNode }) =>
    <button aria-label={label} {...rest}>{children}</button>,
  ConfirmDialog: () => null,
  EmptyState: ({ title, description }: { title: string; description: string }) => (
    <div>
      <strong>{title}</strong>
      <p>{description}</p>
    </div>
  ),
}))

const key = 'iii::ide::timeline-view-mode'
const files: TurnFileHead[] = [
  { path: '/repo/src/nested/new.ts', kind: 'created', agent: { session_id: 'child', name: 'Builder' } },
  { path: '/repo/src/gone.ts', kind: 'deleted' },
  { path: '/elsewhere/src/new.ts', kind: 'modified' },
]
const props: ComponentProps<typeof TimelineTab> = {
  turns: [
    { turn_id: 'new-turn', title: 'New task', started_at: 2, ended_at: 3, file_count: 3, files },
    { turn_id: 'old-turn', title: 'Old task', started_at: 0, ended_at: 1, file_count: 1, files: [files[0]] },
  ],
  root: '/repo', hasSession: true, runningTurnId: null, activeTurnId: 'new-turn',
  activePath: 'src/nested/new.ts', reverting: null, note: null,
  onRefresh: vi.fn(), onOpenFile: vi.fn(), onOpenWorkingFile: vi.fn(),
  onRevertTurn: vi.fn(), onRevertFile: vi.fn(),
}
function storage(mode: string) {
  const values = new Map([[key, mode]])
  vi.stubGlobal('window', { localStorage: {
    getItem: (name: string) => values.get(name) ?? null,
    setItem: (name: string, value: string) => values.set(name, value),
  } })
}
afterEach(() => vi.unstubAllGlobals())

describe('Timeline file views', () => {
  it('groups relative paths without changing absolute identities or agent metadata', () => {
    const tree = buildChangeTree(files, (file) => relativeToRoot(file.path, '/repo'))
    expect(tree.directories.map((dir) => dir.name)).toEqual(['Outside workspace', 'src'])
    expect(tree.entries).toEqual([])
    const outside = tree.directories[0]
    expect(outside.directories[0].path).toBe('/elsewhere')
    expect(outside.directories[0].directories[0].entries[0]).toBe(files[2])
    expect(tree.directories[1].directories[0].entries[0]).toBe(files[0])
    expect(tree.directories[1].entries[0]).toBe(files[1])
  })

  it.each(['list', 'tree'])('preserves turn order, status, agents and file actions in %s mode', (mode) => {
    storage(mode)
    const html = renderToStaticMarkup(<TimelineTab {...props} />)
    expect(html.indexOf('New task')).toBeLessThan(html.indexOf('Old task'))
    expect(html.match(/class="shui-timeline-turn /g)).toHaveLength(2)
    expect(html.match(/label="Revert this file"|aria-label="Revert this file"/g)).toHaveLength(4)
    expect(html).toContain('changed by Builder')
    expect(html).toContain('data-status="deleted"')
    expect(html).toContain('shui-scm-row active')
    expect(html).toContain('shui-scm-row outside')
    expect(html).toContain('disabled="" title="/elsewhere/src/new.ts (outside this folder)"')
    expect(html).toContain(mode === 'list' ? 'View as Tree' : 'View as List')
    if (mode === 'tree') {
      expect(html.match(/class="shui-scm-folder"/g)).toHaveLength(6)
      expect(html).toContain('padding-left:34px')
      expect(html).toContain('Outside workspace')
      expect(html).toContain('<span>../elsewhere/src</span>')
      expect(html).toContain('title="/elsewhere/src"')
      expect(html).not.toContain('<span class="dir">/elsewhere/src</span>')
    } else {
      expect(html).not.toContain('shui-scm-folder')
      expect(html).toContain('<span class="dir">src/nested</span>')
    }
  })

  it('separates matching internal and external paths, including root-level files', () => {
    const entries: TurnFileHead[] = [
      { path: '/repo/src/a.ts', kind: 'modified' },
      { path: '/src/a.ts', kind: 'deleted' },
      { path: '/a.ts', kind: 'created' },
      { path: '/other/src/a.ts', kind: 'modified' },
      { path: '/repo/Outside workspace/a.ts', kind: 'modified' },
    ]
    const tree = buildChangeTree(entries, (file) => relativeToRoot(file.path, '/repo'))
    const outside = tree.directories.find((dir) => dir.path === '/')!
    expect(outside.entries).toEqual([entries[2]])
    expect(outside.directories.map((dir) => dir.path)).toEqual(['/other', '/src'])
    expect(outside.directories[1].entries[0]).toBe(entries[1])
    expect(outside.directories[0].directories[0].entries[0]).toBe(entries[3])
    expect(tree.directories.find((dir) => dir.path === 'src')!.entries[0]).toBe(entries[0])
    expect(tree.directories.find((dir) => dir.path === 'Outside workspace')!.entries[0]).toBe(entries[4])
  })

  it('builds an external-only tree without empty folder names', () => {
    const tree = buildChangeTree([files[2]], () => null)
    expect(tree.entries).toEqual([])
    expect(tree.directories).toHaveLength(1)
    expect(tree.directories[0].directories[0].name).toBe('elsewhere')
    expect(tree.directories[0].directories[0].directories[0].name).toBe('src')
  })

  it.each([
    ['/work/ide/src/a.ts', '/work/harness', '../ide/src/a.ts'],
    ['/work/ide/a.ts', '/work/harness/', '../ide/a.ts'],
    ['/other/a.ts', '/work/harness', '../../other/a.ts'],
    ['/work/harness-old/a.ts', '/work/harness', '../harness-old/a.ts'],
    ['/a.ts', '/', 'a.ts'],
    ['/work/a.ts', '/work/harness', '../a.ts'],
  ])('shows %s relative to %s', (path, root, expected) => {
    expect(relativeDisplayPath(path, root)).toBe(expected)
  })

  it('compacts external folder chains and retains absolute tooltips and file identities', () => {
    const file = { path: '/home/dev/ide/ui/src/a.ts', kind: 'modified' }
    const tree = buildChangeTree([file], () => null, '/home/dev/harness')
    const compact = tree.directories[0].directories[0]
    expect(compact.name).toBe('../ide/ui/src')
    expect(compact.title).toBe('/home/dev/ide/ui/src')
    expect(compact.entries[0]).toBe(file)
    expect(compact.directories).toEqual([])
  })

  it('stops compacting at branches and directories containing files', () => {
    const entries = [
      { path: '/work/ide/a.ts' },
      { path: '/work/ide/ui/b.ts' },
      { path: '/work/other/c.ts' },
    ]
    const tree = buildChangeTree(entries, () => null, '/work/harness')
    const parent = tree.directories[0].directories[0]
    expect(parent.name).toBe('..')
    expect(parent.directories.map((dir) => dir.name)).toEqual(['ide', 'other'])
    expect(parent.directories[0].entries[0]).toBe(entries[0])
    expect(parent.directories[0].directories[0].entries[0]).toBe(entries[1])
  })

  it('keeps the Timeline preference separate from Source Control', () => {
    storage('tree')
    expect(readScmViewMode(key)).toBe('tree')
    expect(readScmViewMode()).toBe('list')
    writeScmViewMode('list', key)
    writeScmViewMode('tree')
    expect(readScmViewMode(key)).toBe('list')
    expect(readScmViewMode()).toBe('tree')
  })

  it('retains the empty states in tree mode', () => {
    storage('tree')
    expect(renderToStaticMarkup(<TimelineTab {...props} hasSession={false} />)).toContain('beside a chat')
    expect(renderToStaticMarkup(<TimelineTab {...props} turns={[]} />)).toContain('No turn has changed files yet.')
  })
})
