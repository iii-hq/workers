import { ChevronDown, ChevronRight, Folder, FolderOpen } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { buildChangeTree, type ChangeDirectory, type ScmViewMode } from './scm-view'

type PathEntry = { path: string }
type RenderEntry<T extends PathEntry> = (entry: T, depth: number) => ReactNode
/** Hover actions for a folder row, given every entry under it (subfolders included). */
type RenderDirectoryActions<T extends PathEntry> = (entries: T[], directory: ChangeDirectory<T>) => ReactNode

/** One tree level: a folder's caret and the gap after it. A file keeps the
    caret's place, so its icon lines up with its sibling folders' icons and a
    child's caret sits under its parent's icon. */
export const TREE_INDENT = 20
/** A folder row's left padding at `depth`; a file row's is `TREE_INDENT` more. */
export const treeInset = (depth: number) => 6 + depth * TREE_INDENT

/** Every entry under a directory, depth first. */
export function directoryEntries<T extends PathEntry>(directory: ChangeDirectory<T>): T[] {
  return [...directory.directories.flatMap((child) => directoryEntries(child)), ...directory.entries]
}

/** Shared file layout; all actions and original file identities stay with the caller. */
export function ChangeEntries<T extends PathEntry>({ entries, mode, renderEntry, renderDirectoryActions, getPath, outsideRoot, defaultOpen = true }: {
  entries: readonly T[]
  mode: ScmViewMode
  renderEntry: RenderEntry<T>
  renderDirectoryActions?: RenderDirectoryActions<T>
  getPath?: (entry: T) => string | null
  outsideRoot?: string
  /** Folders start open; a new `key` re-applies it to every folder. */
  defaultOpen?: boolean
}) {
  const tree = useMemo(() => mode === 'tree' ? buildChangeTree(entries, getPath, outsideRoot) : null, [entries, mode, getPath, outsideRoot])
  if (!tree) return <>{entries.map((entry) => renderEntry(entry, 0))}</>
  return <DirectoryEntries directory={tree} depth={0} renderEntry={renderEntry} renderDirectoryActions={renderDirectoryActions} defaultOpen={defaultOpen} />
}

function DirectoryEntries<T extends PathEntry>({ directory, depth, renderEntry, renderDirectoryActions, defaultOpen }: {
  directory: ChangeDirectory<T>
  depth: number
  defaultOpen: boolean
  renderEntry: RenderEntry<T>
  renderDirectoryActions?: RenderDirectoryActions<T>
}) {
  return (
    <ul className="shui-scm-tree-list">
      {directory.directories.map((child) => (
        <DirectoryRow key={child.path} directory={child} depth={depth} renderEntry={renderEntry} renderDirectoryActions={renderDirectoryActions} defaultOpen={defaultOpen} />
      ))}
      {directory.entries.map((entry) => <li key={entry.path}>{renderEntry(entry, depth)}</li>)}
    </ul>
  )
}

function DirectoryRow<T extends PathEntry>({ directory, depth, renderEntry, renderDirectoryActions, defaultOpen }: {
  directory: ChangeDirectory<T>
  depth: number
  defaultOpen: boolean
  renderEntry: RenderEntry<T>
  renderDirectoryActions?: RenderDirectoryActions<T>
}) {
  const [open, setOpen] = useState(defaultOpen)
  const Chevron = open ? ChevronDown : ChevronRight
  const Icon = open ? FolderOpen : Folder
  const toggle = (
    <button
      type="button"
      className="shui-scm-folder"
      style={{ paddingLeft: treeInset(depth) }}
      title={directory.title ?? directory.path}
      aria-expanded={open}
      onClick={() => setOpen((value) => !value)}
    >
      <Chevron aria-hidden />
      <Icon aria-hidden />
      <span>{directory.name}</span>
    </button>
  )
  const actions = renderDirectoryActions ? renderDirectoryActions(directoryEntries(directory), directory) : null
  return (
    <li>
      {actions ? (
        <div className="shui-scm-row shui-scm-folder-row">
          {toggle}
          <span className="shui-scm-row-actions">{actions}</span>
          <span aria-hidden />
        </div>
      ) : toggle}
      {open ? <DirectoryEntries directory={directory} depth={depth + 1} renderEntry={renderEntry} renderDirectoryActions={renderDirectoryActions} defaultOpen={defaultOpen} /> : null}
    </li>
  )
}
