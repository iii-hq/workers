import { ChevronRight, CornerLeftUp, File, Folder } from 'lucide-react'
import { useMemo } from 'react'
import type { FlatTree } from './coder'

interface WorkspaceBrowserProps {
  tree: FlatTree | null
  path: string
  rootLabel: string
  onOpenFolder: (relPath: string) => void
  onOpenFile: (relPath: string) => void
}

interface BrowserEntry {
  name: string
  relPath: string
  dir: boolean
}

export function workspaceBrowserEntries(
  tree: FlatTree | null,
  path: string,
): BrowserEntry[] {
  if (!tree) return []
  const prefix = path === '' ? '' : `${path}/`
  const seen = new Map<string, BrowserEntry>()
  for (const entry of tree.paths) {
    const relPath = entry.endsWith('/') ? entry.slice(0, -1) : entry
    if (!relPath.startsWith(prefix)) continue
    const rest = relPath.slice(prefix.length)
    if (rest === '' || rest.includes('/')) continue
    seen.set(rest, {
      name: rest,
      relPath,
      dir: entry.endsWith('/'),
    })
  }
  return [...seen.values()].sort((left, right) => {
    if (left.dir !== right.dir) return left.dir ? -1 : 1
    return left.name.localeCompare(right.name)
  })
}

export function WorkspaceBrowser({
  tree,
  path,
  rootLabel,
  onOpenFolder,
  onOpenFile,
}: WorkspaceBrowserProps) {
  const entries = useMemo(
    () => workspaceBrowserEntries(tree, path),
    [tree, path],
  )
  const segments = path === '' ? [] : path.split('/')
  const parent = segments.slice(0, -1).join('/')

  return (
    <div className="shui-browser">
      <nav className="shui-browser-crumbs" aria-label="Workspace path">
        <button
          type="button"
          className="shui-browser-crumb"
          onClick={() => onOpenFolder('')}
        >
          {rootLabel}
        </button>
        {segments.map((segment, index) => {
          const target = segments.slice(0, index + 1).join('/')
          return (
            <span className="shui-browser-crumb-group" key={target}>
              <ChevronRight aria-hidden />
              <button
                type="button"
                className="shui-browser-crumb"
                onClick={() => onOpenFolder(target)}
              >
                {segment}
              </button>
            </span>
          )
        })}
      </nav>
      <div className="shui-browser-list">
        {path !== '' ? (
          <button
            type="button"
            className="shui-browser-entry"
            onClick={() => onOpenFolder(parent)}
          >
            <CornerLeftUp aria-hidden className="shui-browser-entry-icon" />
            <span>..</span>
          </button>
        ) : null}
        {entries.map((entry) => (
          <button
            type="button"
            key={entry.relPath}
            className={`shui-browser-entry${entry.dir ? ' dir' : ''}`}
            onClick={() =>
              entry.dir
                ? onOpenFolder(entry.relPath)
                : onOpenFile(entry.relPath)
            }
          >
            {entry.dir ? (
              <Folder aria-hidden className="shui-browser-entry-icon" />
            ) : (
              <File aria-hidden className="shui-browser-entry-icon" />
            )}
            <span>{entry.name}</span>
          </button>
        ))}
        {entries.length === 0 ? (
          <span className="shui-browser-empty t-ghost">
            {tree === null ? 'loading workspace…' : 'this folder is empty'}
          </span>
        ) : null}
      </div>
    </div>
  )
}
