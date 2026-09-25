import { ChevronDown, ChevronRight, Folder, FolderOpen } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { buildChangeTree, type ChangeDirectory, type ScmViewMode } from './scm-view'

type PathEntry = { path: string }
type RenderEntry<T extends PathEntry> = (entry: T, depth: number) => ReactNode
/** Hover actions for a folder row, given every entry under it (subfolders included). */
type RenderDirectoryActions<T extends PathEntry> = (entries: T[], directory: ChangeDirectory<T>) => ReactNode

/** Every entry under a directory, depth first. */
export function directoryEntries<T extends PathEntry>(directory: ChangeDirectory<T>): T[] {
  return [...directory.directories.flatMap((child) => directoryEntries(child)), ...directory.entries]
}

/** Shared file layout; all actions and original file identities stay with the caller. */
export function ChangeEntries<T extends PathEntry>({ entries, mode, renderEntry, renderDirectoryActions, getPath, outsideRoot }: {
  entries: readonly T[]
  mode: ScmViewMode
  renderEntry: RenderEntry<T>
  renderDirectoryActions?: RenderDirectoryActions<T>
  getPath?: (entry: T) => string | null
  outsideRoot?: string
}) {
  const tree = useMemo(() => mode === 'tree' ? buildChangeTree(entries, getPath, outsideRoot) : null, [entries, mode, getPath, outsideRoot])
  if (!tree) return <>{entries.map((entry) => renderEntry(entry, 0))}</>
  return <DirectoryEntries directory={tree} depth={0} renderEntry={renderEntry} renderDirectoryActions={renderDirectoryActions} />
}

function DirectoryEntries<T extends PathEntry>({ directory, depth, renderEntry, renderDirectoryActions }: {
  directory: ChangeDirectory<T>
  depth: number
  renderEntry: RenderEntry<T>
  renderDirectoryActions?: RenderDirectoryActions<T>
}) {
  return (
    <ul className="shui-scm-tree-list">
      {directory.directories.map((child) => (
        <DirectoryRow key={child.path} directory={child} depth={depth} renderEntry={renderEntry} renderDirectoryActions={renderDirectoryActions} />
      ))}
      {directory.entries.map((entry) => <li key={entry.path}>{renderEntry(entry, depth)}</li>)}
    </ul>
  )
}

function DirectoryRow<T extends PathEntry>({ directory, depth, renderEntry, renderDirectoryActions }: {
  directory: ChangeDirectory<T>
  depth: number
  renderEntry: RenderEntry<T>
  renderDirectoryActions?: RenderDirectoryActions<T>
}) {
  const [open, setOpen] = useState(true)
  const Chevron = open ? ChevronDown : ChevronRight
  const Icon = open ? FolderOpen : Folder
  const toggle = (
    <button
      type="button"
      className="shui-scm-folder"
      style={{ paddingLeft: 6 + depth * 14 }}
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
      {open ? <DirectoryEntries directory={directory} depth={depth + 1} renderEntry={renderEntry} renderDirectoryActions={renderDirectoryActions} /> : null}
    </li>
  )
}
