import { ChevronDown, ChevronRight, Folder, FolderOpen } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { buildChangeTree, type ChangeDirectory, type ScmViewMode } from './scm-view'

type PathEntry = { path: string }
type RenderEntry<T extends PathEntry> = (entry: T, depth: number) => ReactNode

/** Shared file layout; all actions and original file identities stay with the caller. */
export function ChangeEntries<T extends PathEntry>({ entries, mode, renderEntry, getPath }: {
  entries: readonly T[]
  mode: ScmViewMode
  renderEntry: RenderEntry<T>
  getPath?: (entry: T) => string | null
}) {
  const tree = useMemo(() => mode === 'tree' ? buildChangeTree(entries, getPath) : null, [entries, mode, getPath])
  if (!tree) return <>{entries.map((entry) => renderEntry(entry, 0))}</>
  return <DirectoryEntries directory={tree} depth={0} renderEntry={renderEntry} />
}

function DirectoryEntries<T extends PathEntry>({ directory, depth, renderEntry }: {
  directory: ChangeDirectory<T>
  depth: number
  renderEntry: RenderEntry<T>
}) {
  return (
    <ul className="shui-scm-tree-list">
      {directory.directories.map((child) => (
        <DirectoryRow key={child.path} directory={child} depth={depth} renderEntry={renderEntry} />
      ))}
      {directory.entries.map((entry) => <li key={entry.path}>{renderEntry(entry, depth)}</li>)}
    </ul>
  )
}

function DirectoryRow<T extends PathEntry>({ directory, depth, renderEntry }: {
  directory: ChangeDirectory<T>
  depth: number
  renderEntry: RenderEntry<T>
}) {
  const [open, setOpen] = useState(true)
  const Chevron = open ? ChevronDown : ChevronRight
  const Icon = open ? FolderOpen : Folder
  return (
    <li>
      <button
        type="button"
        className="shui-scm-folder"
        style={{ paddingLeft: 6 + depth * 14 }}
        title={directory.path}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <Chevron aria-hidden />
        <Icon aria-hidden />
        <span>{directory.name}</span>
      </button>
      {open ? <DirectoryEntries directory={directory} depth={depth + 1} renderEntry={renderEntry} /> : null}
    </li>
  )
}
