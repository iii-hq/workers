import { describe, expect, it, vi } from 'vitest'
import { inMenuScope } from '../ChangesTree'
import type { ContextMenuItem } from '../ContextMenu'
import type { ChangeRow } from '../commit-tree'
import type { GitComparisonEntry } from '../git'
import type { GitStash } from '../git-log'
import { type ChangeMenuContext, changeMenu, stashMenu } from '../scm-menus'

// The shared components only exist inside the console; these tests read pure helpers.
vi.mock('@iii-dev/console-ui', () => ({}))

function entry(path: string, status: GitComparisonEntry['status'] = 'modified'): GitComparisonEntry {
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

const fileRow = (e: GitComparisonEntry): ChangeRow => ({
  kind: 'file',
  key: e.path,
  group: e.status === 'untracked' ? 'unversioned' : 'changes',
  depth: 1,
  entry: e,
  dir: '',
})

function context(busy = false) {
  const calls: Array<[string, unknown]> = []
  const record =
    (name: string) =>
    (...args: unknown[]) =>
      calls.push([name, args.length === 1 ? args[0] : args])
  const ctx: ChangeMenuContext = {
    root: '/repo',
    busy,
    commit: record('commit'),
    rollback: record('rollback'),
    stash: record('stash'),
    open: record('open'),
    jump: record('jump'),
    copy: record('copy'),
    remove: record('remove'),
    add: record('add'),
    ignore: record('ignore'),
    savePatch: record('savePatch'),
    copyPatch: record('copyPatch'),
    refresh: record('refresh'),
    compare: record('compare'),
    history: record('history'),
  }
  return { ctx, calls }
}

/** `label` (disabled marked `!`), separators as `---`, a submenu as `label > [..]`;
    the head row naming the target is left out (see its own test). */
function shape(items: readonly ContextMenuItem[]): unknown[] {
  return items
    .filter((item) => item.id !== 'head' && item.id !== 'sep:head')
    .map((item) =>
      item.type === 'separator'
        ? '---'
        : item.type === 'submenu'
          ? { [`${item.label}${item.disabled ? '!' : ''}`]: shape(item.items) }
          : item.type === 'label'
            ? item.label
            : `${item.label}${item.disabled ? '!' : ''}`,
    )
}

function select(items: readonly ContextMenuItem[], ...path: string[]): void {
  const [label, ...rest] = path
  const item = items.find(
    (candidate) => candidate.type !== 'separator' && 'label' in candidate && candidate.label === label,
  )
  if (!item) throw new Error(`no ${label}`)
  if (item.type === 'submenu') {
    select(item.items, ...rest)
    return
  }
  if (item.type === 'separator' || item.type === 'label') throw new Error(`${label} is not an action`)
  item.onSelect()
}

/** The first row's label and detail. */
function head(items: readonly ContextMenuItem[]): [string, string | undefined] {
  const [first, second] = items
  if (first?.type !== 'label' || second?.type !== 'separator') throw new Error('no head row')
  return [first.label, first.detail]
}

describe('the head row', () => {
  it('names the file, folder, group or stash the menu acts on', () => {
    const { ctx } = context()
    expect(head(changeMenu(fileRow(entry('src/page/a.ts')), ctx))).toEqual(['a.ts', 'src/page'])
    expect(head(changeMenu(fileRow(entry('a.ts')), ctx))).toEqual(['a.ts', undefined])
    const files = [entry('out/a.js', 'untracked'), entry('out/b.js', 'untracked')]
    const folder: ChangeRow = {
      kind: 'folder',
      key: 'unversioned:out/',
      group: 'unversioned',
      depth: 1,
      label: 'out',
      path: 'out',
      open: true,
      entries: files,
    }
    expect(head(changeMenu(folder, ctx))).toEqual(['out', '2 files'])
    const group: ChangeRow = {
      kind: 'group',
      key: 'changes',
      group: 'changes',
      depth: 0,
      label: 'Changes',
      open: true,
      entries: [files[0]],
    }
    expect(head(changeMenu(group, ctx))).toEqual(['Changes', '1 file'])
    const noop = () => undefined
    const stash: GitStash = { sha: 'abc', ref: 'stash@{1}', message: 'wip', time: 0, branch: 'main' }
    expect(
      head(stashMenu(stash, { busy: false, apply: noop, unstash: noop, drop: noop, clear: noop, showDiff: noop })),
    ).toEqual(['wip', 'stash@{1}'])
  })
})

describe('the rows a menu marks', () => {
  it('marks every row under a group or a folder, not the target itself or a sibling', () => {
    expect(inMenuScope('changes:src/a.ts', 'changes')).toBe(true)
    expect(inMenuScope('unversioned:a.ts', 'changes')).toBe(false)
    expect(inMenuScope('changes:src/a.ts', 'changes:src/')).toBe(true)
    expect(inMenuScope('changes:src/lib/b.ts', 'changes:src/')).toBe(true)
    expect(inMenuScope('changes:srcx/a.ts', 'changes:src/')).toBe(false)
    expect(inMenuScope('changes:src/', 'changes:src/')).toBe(false)
    // A file's menu acts on that file alone.
    expect(inMenuScope('changes:src/a.ts.map', 'changes:src/a.ts')).toBe(false)
    expect(inMenuScope('changes:src/a.ts', null)).toBe(false)
  })
})

describe('a change row menu', () => {
  it('offers a tracked file every action WebStorm has that applies here', () => {
    const { ctx } = context()
    expect(shape(changeMenu(fileRow(entry('src/a.ts')), ctx))).toEqual([
      'Commit file…',
      'Rollback…',
      '---',
      'Show diff',
      'Show diff in a new tab',
      'Jump to source',
      { 'Copy path/reference': ['Absolute path', 'Path from root', 'File name'] },
      '---',
      'Delete…',
      'Add to VCS!',
      'Add to .gitignore!',
      '---',
      'Create patch from local changes',
      'Copy as patch to clipboard',
      'Stash changes…',
      '---',
      'Refresh',
      '---',
      { Git: ['Compare with…', 'Show history'] },
    ])
  })

  it('acts on the file it was opened on', () => {
    const { ctx, calls } = context()
    const file = entry('src/a.ts')
    const items = changeMenu(fileRow(file), ctx)
    select(items, 'Show diff')
    select(items, 'Show diff in a new tab')
    select(items, 'Jump to source')
    select(items, 'Copy path/reference', 'Absolute path')
    select(items, 'Copy path/reference', 'File name')
    select(items, 'Git', 'Show history')
    expect(calls).toEqual([
      ['open', [file, false]],
      ['open', [file, true]],
      ['jump', 'src/a.ts'],
      ['copy', '/repo/src/a.ts'],
      ['copy', 'a.ts'],
      ['history', ['src/a.ts']],
    ])
  })

  it('lets an unversioned file be added or ignored, not rolled back or traced', () => {
    const { ctx, calls } = context()
    const file = entry('notes/new file.md', 'untracked')
    const items = changeMenu(fileRow(file), ctx)
    expect(shape(items)).toContain('Rollback…!')
    expect(shape(items)).toContainEqual({ Git: ['Compare with…!', 'Show history!'] })
    select(items, 'Add to VCS')
    select(items, 'Add to .gitignore')
    expect(calls).toEqual([
      ['add', [file]],
      ['ignore', ['notes/new file.md']],
    ])
  })

  it('cannot open or delete a deleted file again', () => {
    const { ctx } = context()
    const items = shape(changeMenu(fileRow(entry('gone.ts', 'deleted')), ctx))
    expect(items).toContain('Jump to source!')
    expect(items).toContain('Delete…!')
  })

  it('ignores an unversioned folder whole and acts on every file under it', () => {
    const { ctx, calls } = context()
    const files = [entry('out/a.js', 'untracked'), entry('out/b.js', 'untracked')]
    const row: ChangeRow = {
      kind: 'folder',
      key: 'unversioned:out',
      group: 'unversioned',
      depth: 1,
      label: 'out',
      path: 'out',
      open: true,
      entries: files,
    }
    const items = changeMenu(row, ctx)
    expect(shape(items)).toEqual([
      'Commit files…',
      'Rollback…!',
      '---',
      { 'Copy path/reference': ['Absolute path', 'Path from root'] },
      '---',
      'Add to VCS',
      'Add to .gitignore',
      '---',
      'Create patch from local changes',
      'Copy as patch to clipboard',
      'Stash changes…',
      '---',
      'Refresh',
      '---',
      { Git: ['Show history!'] },
    ])
    select(items, 'Add to .gitignore')
    select(items, 'Commit files…')
    expect(calls).toEqual([
      ['ignore', ['out/']],
      ['commit', files],
    ])
  })

  it('rolls back only the tracked files of a group, and has no path to copy', () => {
    const { ctx, calls } = context()
    const tracked = [entry('a.ts'), entry('b.ts', 'added')]
    const files = [...tracked, entry('new.md', 'untracked')]
    const row: ChangeRow = {
      kind: 'group',
      key: 'changes',
      group: 'changes',
      depth: 0,
      label: 'Changes',
      open: true,
      entries: files,
    }
    const items = changeMenu(row, ctx)
    const labels = shape(items)
    expect(labels).not.toContainEqual(expect.objectContaining({ 'Copy path/reference': expect.anything() }))
    expect(labels).not.toContainEqual(expect.objectContaining({ Git: expect.anything() }))
    select(items, 'Rollback…')
    select(items, 'Copy as patch to clipboard')
    expect(calls).toEqual([
      ['rollback', tracked],
      ['copyPatch', files],
    ])
  })

  it('holds back what writes while another operation runs', () => {
    const { ctx } = context(true)
    const labels = shape(changeMenu(fileRow(entry('a.ts')), ctx))
    for (const label of ['Rollback…!', 'Delete…!', 'Stash changes…!']) expect(labels).toContain(label)
    // Reading and copying never wait.
    for (const label of ['Show diff', 'Copy as patch to clipboard', 'Refresh']) expect(labels).toContain(label)
  })
})

describe('a stash row menu', () => {
  const stash: GitStash = { sha: 'abc', ref: 'stash@{1}', message: 'wip', time: 0, branch: 'main' }

  it("lists WebStorm's stash actions and runs them on that stash", () => {
    const calls: unknown[] = []
    const items = stashMenu(stash, {
      busy: false,
      apply: (target, pop) => calls.push(['apply', target.ref, pop]),
      unstash: (target) => calls.push(['unstash', target.ref]),
      drop: (target) => calls.push(['drop', target.ref]),
      clear: () => calls.push(['clear']),
      showDiff: (target, pin) => calls.push(['diff', target.ref, pin]),
    })
    expect(shape(items)).toEqual([
      'Pop',
      'Apply',
      'Unstash…',
      '---',
      'Drop…',
      'Clear…',
      '---',
      'Show diff',
      'Show diff in a new tab',
    ])
    for (const label of ['Pop', 'Apply', 'Unstash…', 'Drop…', 'Clear…', 'Show diff', 'Show diff in a new tab']) {
      select(items, label)
    }
    expect(calls).toEqual([
      ['apply', 'stash@{1}', true],
      ['apply', 'stash@{1}', false],
      ['unstash', 'stash@{1}'],
      ['drop', 'stash@{1}'],
      ['clear'],
      ['diff', 'stash@{1}', false],
      ['diff', 'stash@{1}', true],
    ])
  })

  it('keeps Show diff open while an operation runs', () => {
    const noop = () => undefined
    const items = stashMenu(stash, { busy: true, apply: noop, unstash: noop, drop: noop, clear: noop, showDiff: noop })
    expect(shape(items)).toEqual([
      'Pop!',
      'Apply!',
      'Unstash…!',
      '---',
      'Drop…!',
      'Clear…!',
      '---',
      'Show diff',
      'Show diff in a new tab',
    ])
  })
})
