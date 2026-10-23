/* The files a commit changed, or that differ from the working tree, as a
   tree under the IDE's folder or as a flat list. Files outside it gather
   under their own group, their folders named from the IDE's (`../ide/src`),
   as in the Timeline. A click selects a file, for the actions over the
   list; a double click or Enter opens its diff, and a right click (or the
   menu key) its menu. Windowed: a merge or a comparison against an old ref
   reaches thousands of files, and only the rows in view mount. */

import { ChevronDown, ChevronRight, Folder, FolderOpen } from 'lucide-react'
import { useMemo, useState } from 'react'
import { TREE_INDENT, treeInset } from './ChangeEntries'
import type { ContextMenuAnchor } from './ContextMenu'
import { statusLetter, statusTitle } from './git-actions'
import type { CommitFile } from './git-log-window'
import { basename, dirname } from './paths'
import { buildChangeTree, type ChangeDirectory } from './scm-view'
import { VirtualList } from './VirtualList'

type Entry = { path: string; file: CommitFile }
type Row = { dir: ChangeDirectory<Entry>; depth: number; open: boolean } | { entry: Entry; depth: number }

/** The tree as rows, folders before files at each level, as ChangeEntries lays it out. */
function treeRows(directory: ChangeDirectory<Entry>, isOpen: (path: string) => boolean, depth = 0, rows: Row[] = []) {
  for (const dir of directory.directories) {
    const open = isOpen(dir.path)
    rows.push({ dir, depth, open })
    if (open) treeRows(dir, isOpen, depth + 1, rows)
  }
  for (const entry of directory.entries) rows.push({ entry, depth })
  return rows
}

const keyOf = (row: Row) => ('dir' in row ? `d:${row.dir.path}` : `f:${row.entry.file.path}`)

export function GitFileList({
  files,
  prefix,
  top,
  grouped = true,
  open = true,
  narrow = false,
  selected = null,
  onSelect,
  onMenu,
  onOpen,
}: {
  files: readonly CommitFile[]
  /** The IDE's folder below the repository's top ('' at the top). */
  prefix: string
  /** The worktree's absolute top, when known: names the outside folders. */
  top: string | null
  /** A tree of folders; else a flat list, each file beside its folder. */
  grouped?: boolean
  /** Folders start open; a new `key` re-applies it to every folder. */
  open?: boolean
  /** Touch-sized rows (`.shui-git-window[data-narrow]`). */
  narrow?: boolean
  /** The selected file's path. */
  selected?: string | null
  onSelect?(file: CommitFile): void
  onMenu?(file: CommitFile, anchor: ContextMenuAnchor): void
  onOpen(file: CommitFile): void
}) {
  const base = top ?? ''
  const outsideRoot = `${base}/${prefix.replace(/\/$/, '')}`
  // The outside group reads absolute paths.
  const entries = useMemo(() => files.map((file) => ({ path: `${base}/${file.path}`, file })), [files, base])
  const tree = useMemo(
    () => (grouped ? buildChangeTree(entries, relOf, outsideRoot) : null),
    [grouped, entries, outsideRoot],
  )
  // Folders toggled away from `open`, by path.
  const [flipped, setFlipped] = useState<ReadonlySet<string>>(new Set())
  const rows = useMemo<Row[]>(
    () =>
      tree === null
        ? entries.map((entry) => ({ entry, depth: 0 }))
        : treeRows(tree, (path) => flipped.has(path) !== open),
    [tree, entries, flipped, open],
  )
  const toggle = (path: string) =>
    setFlipped((current) => {
      const next = new Set(current)
      if (!next.delete(path)) next.add(path)
      return next
    })
  // The row holding focus stays mounted when it scrolls out of the window.
  const [focusedKey, setFocusedKey] = useState<string | null>(null)
  const focusedIndex = focusedKey === null ? null : rows.findIndex((row) => keyOf(row) === focusedKey)

  return (
    <VirtualList
      rows={rows}
      rowHeight={narrow ? 44 : 24}
      rowKey={keyOf}
      className="shui-git-file-list"
      role="list"
      aria-label="Changed files"
      keepIndex={focusedIndex}
      renderRow={(row) => (
        // The nesting the windowed rows no longer have, as a level (ARIA 1.2
        // lists aria-level on listitem). A <li> cannot sit in VirtualList's row.
        // biome-ignore lint/a11y/useSemanticElements lint/a11y/useAriaPropsSupportedByRole: see above
        <div role="listitem" aria-level={row.depth + 1}>
          {fileRow(row)}
        </div>
      )}
    />
  )

  function fileRow(row: Row) {
    const key = keyOf(row)
    const focus = {
      onFocus: () => setFocusedKey(key),
      // The browser window losing focus blurs the row but leaves it the
      // active element: keep it mounted for when the window comes back.
      onBlur: (event: React.FocusEvent) => {
        if (document.activeElement !== event.currentTarget)
          setFocusedKey((current) => (current === key ? null : current))
      },
    }
    if ('dir' in row) {
      const { dir, depth, open: expanded } = row
      const Chevron = expanded ? ChevronDown : ChevronRight
      const Icon = expanded ? FolderOpen : Folder
      return (
        <button
          type="button"
          className="shui-scm-folder"
          style={{ paddingLeft: treeInset(depth) }}
          title={dir.title ?? dir.path}
          aria-expanded={expanded}
          onClick={() => toggle(dir.path)}
          {...focus}
        >
          <Chevron aria-hidden />
          <Icon aria-hidden />
          <span>{dir.name}</span>
        </button>
      )
    }
    const { file } = row.entry
    const folder = grouped ? '' : dirname(file.view)
    return (
      <button
        type="button"
        className="shui-git-file"
        aria-current={file.path === selected || undefined}
        style={{ paddingLeft: grouped ? treeInset(row.depth) + TREE_INDENT : 8 }}
        title={file.from ? `${file.from} → ${file.path}` : file.path}
        onClick={() => onSelect?.(file)}
        onFocus={() => {
          focus.onFocus()
          onSelect?.(file)
        }}
        onBlur={focus.onBlur}
        onDoubleClick={() => onOpen(file)}
        onContextMenu={(event) => {
          if (!onMenu) return
          event.preventDefault()
          // From the keyboard the event has no pointer: open under the row.
          const rect = event.currentTarget.getBoundingClientRect()
          const keyed = event.clientX === 0 && event.clientY === 0
          onMenu(file, keyed ? { x: rect.left + 16, y: rect.bottom } : { x: event.clientX, y: event.clientY })
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter') onOpen(file)
        }}
      >
        <span className="shui-git-file-status" data-status={file.status} title={statusTitle(file.status)}>
          {statusLetter(file.status)}
        </span>
        <span className="shui-git-file-name">{basename(file.path)}</span>
        {folder !== '' ? <span className="shui-git-file-dir">{folder}</span> : null}
      </button>
    )
  }
}

const relOf = (entry: { file: CommitFile }) => entry.file.rel
