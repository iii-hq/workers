import type { ComponentProps, ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TimelineTab } from '../TimelineTab'
import { buildChangeTree, readScmViewMode, writeScmViewMode } from '../scm-view'
import { relativeToRoot, type TurnFileHead } from '../turns'

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({
  IconButton: ({ label, children, ...rest }: { label: string; children: ReactNode }) =>
    <button aria-label={label} {...rest}>{children}</button>,
  ConfirmDialog: () => null,
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
    expect(tree.directories.map((dir) => dir.name)).toEqual(['src'])
    expect(tree.entries).toEqual([files[2]])
    expect(tree.directories[0].directories[0].entries[0]).toBe(files[0])
    expect(tree.directories[0].entries[0]).toBe(files[1])
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
      expect(html.match(/class="shui-scm-folder"/g)).toHaveLength(4)
      expect(html).toContain('padding-left:34px')
      expect(html).not.toContain('title="/elsewhere"')
    } else {
      expect(html).not.toContain('shui-scm-folder')
      expect(html).toContain('<span class="dir">src/nested</span>')
    }
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
