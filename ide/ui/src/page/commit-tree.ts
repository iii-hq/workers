/* The Commit panel's change tree as flat rows: the Changes and Unversioned
   files groups, their folders (a chain of single-child folders folds into one
   row, `tech-specs/2026-06-agentic`, the way IntelliJ shows it) and the files.
   Pure, so the view only renders rows and the tests read them. */

import type { GitComparisonEntry } from './git'
import { buildChangeTree, type ChangeDirectory } from './scm-view'

export type ChangeGroupId = 'changes' | 'unversioned'

export interface ChangeGroup {
  id: ChangeGroupId
  label: string
  entries: readonly GitComparisonEntry[]
}

interface RowBase {
  key: string
  group: ChangeGroupId
  depth: number
}

export type ChangeRow =
  | (RowBase & { kind: 'group'; label: string; open: boolean; entries: GitComparisonEntry[] })
  | (RowBase & { kind: 'folder'; label: string; path: string; open: boolean; entries: GitComparisonEntry[] })
  | (RowBase & { kind: 'file'; entry: GitComparisonEntry; dir: string })

export interface ChangeRowOptions {
  /** Folders (IntelliJ's "Group by: Directory"); off lists files with their folder beside them. */
  byDirectory: boolean
  /** Whether a group or folder row is expanded; `fallback` is its default. */
  isOpen(key: string, fallback: boolean): boolean
}

/** Group rows stay open; a folder opens by default only among the changes,
    so a few hundred unversioned files arrive folded. */
export function defaultOpen(kind: 'group' | 'folder', group: ChangeGroupId): boolean {
  return kind === 'group' || group === 'changes'
}

export function rowKey(group: ChangeGroupId, path = ''): string {
  return path === '' ? group : `${group}:${path}`
}

function entriesUnder<T extends { path: string }>(directory: ChangeDirectory<T>): T[] {
  return [...directory.directories.flatMap((child) => entriesUnder(child)), ...directory.entries]
}

function folded<T extends { path: string }>(directory: ChangeDirectory<T>): ChangeDirectory<T> {
  let current = directory
  let name = directory.name
  while (current.entries.length === 0 && current.directories.length === 1) {
    current = current.directories[0]
    name = `${name}/${current.name}`
  }
  return { ...current, name }
}

function dirname(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash === -1 ? '' : path.slice(0, slash)
}

export function changeRows(groups: readonly ChangeGroup[], options: ChangeRowOptions): ChangeRow[] {
  const rows: ChangeRow[] = []
  for (const group of groups) {
    if (group.entries.length === 0) continue
    const groupKey = rowKey(group.id)
    const groupOpen = options.isOpen(groupKey, defaultOpen('group', group.id))
    rows.push({
      kind: 'group',
      key: groupKey,
      group: group.id,
      depth: 0,
      label: group.label,
      open: groupOpen,
      entries: [...group.entries],
    })
    if (!groupOpen) continue
    if (!options.byDirectory) {
      for (const entry of group.entries) {
        rows.push({
          kind: 'file',
          key: rowKey(group.id, entry.path),
          group: group.id,
          depth: 1,
          entry,
          dir: dirname(entry.path),
        })
      }
      continue
    }
    const walk = (directory: ChangeDirectory<GitComparisonEntry>, depth: number) => {
      for (const child of directory.directories) {
        const folder = folded(child)
        const key = rowKey(group.id, `${folder.path}/`)
        const open = options.isOpen(key, defaultOpen('folder', group.id))
        rows.push({
          kind: 'folder',
          key,
          group: group.id,
          depth,
          label: folder.name,
          path: folder.path,
          open,
          entries: entriesUnder(folder),
        })
        if (open) walk(folder, depth + 1)
      }
      for (const entry of directory.entries) {
        rows.push({ kind: 'file', key: rowKey(group.id, entry.path), group: group.id, depth, entry, dir: '' })
      }
    }
    walk(buildChangeTree(group.entries), 1)
  }
  return rows
}

/** Every group and folder key the rows could show, for Expand all / Collapse all. */
export function expandableKeys(groups: readonly ChangeGroup[]): string[] {
  const keys: string[] = []
  for (const group of groups) {
    if (group.entries.length === 0) continue
    keys.push(rowKey(group.id))
    const walk = (directory: ChangeDirectory<GitComparisonEntry>) => {
      for (const child of directory.directories) {
        const folder = folded(child)
        keys.push(rowKey(group.id, `${folder.path}/`))
        walk(folder)
      }
    }
    walk(buildChangeTree(group.entries))
  }
  return keys
}

/** "3 modified, 1 added, 2 unversioned" for a set of entries; "" when empty. */
export function changeSummary(entries: readonly Pick<GitComparisonEntry, 'status'>[]): string {
  const counts = new Map<string, number>()
  const word = (status: GitComparisonEntry['status']) =>
    status === 'untracked' ? 'unversioned' : status === 'modified' ? 'modified' : status
  for (const entry of entries) counts.set(word(entry.status), (counts.get(word(entry.status)) ?? 0) + 1)
  const order = ['modified', 'added', 'deleted', 'renamed', 'unversioned']
  return order
    .filter((name) => counts.has(name))
    .map((name) => `${counts.get(name)} ${name}`)
    .join(', ')
}

/** The paths a commit or a rollback must name: a rename needs both ends. */
export function entryPaths(entries: readonly Pick<GitComparisonEntry, 'path' | 'renameFrom'>[]): string[] {
  return entries.flatMap((entry) => (entry.renameFrom ? [entry.renameFrom, entry.path] : [entry.path]))
}
