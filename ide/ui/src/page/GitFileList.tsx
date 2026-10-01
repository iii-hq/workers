/* The files a commit changed, or that differ from the working tree, as a
   tree under the IDE's folder or as a flat list. Files outside it gather
   under their own group, their folders named from the IDE's (`../ide/src`),
   as in the Timeline. A click selects a file, for the actions over the
   list; a double click or Enter opens its diff. */

import { useMemo } from 'react'
import { ChangeEntries, TREE_INDENT, treeInset } from './ChangeEntries'
import { statusLetter, statusTitle } from './git-actions'
import type { CommitFile } from './git-log-window'
import { basename, dirname } from './paths'

export function GitFileList({
  files,
  prefix,
  top,
  grouped = true,
  open = true,
  selected = null,
  onSelect,
  onOpen,
}: {
  files: readonly CommitFile[]
  /** The IDE's folder below the repository's top ('' at the top). */
  prefix: string
  /** The worktree's absolute top, when known: names the outside folders. */
  top: string | null
  /** A tree of folders; else a flat list, each file beside its folder. */
  grouped?: boolean
  /** Folders start open. */
  open?: boolean
  /** The selected file's path. */
  selected?: string | null
  onSelect?(file: CommitFile): void
  onOpen(file: CommitFile): void
}) {
  const base = top ?? ''
  // Kept while the files are: ChangeEntries rebuilds its tree on new entries.
  const entries = useMemo(() => files.map((file) => ({ path: `${base}/${file.path}`, file })), [files, base])
  return (
    <div className="shui-git-file-list">
      <ChangeEntries
        // The outside group reads absolute paths.
        entries={entries}
        mode={grouped ? 'tree' : 'list'}
        getPath={relOf}
        outsideRoot={`${base}/${prefix.replace(/\/$/, '')}`}
        defaultOpen={open}
        renderEntry={({ file }, depth) => {
          const folder = grouped ? '' : dirname(file.view)
          return (
            <button
              key={file.path}
              type="button"
              className="shui-git-file"
              aria-current={file.path === selected || undefined}
              style={{ paddingLeft: grouped ? treeInset(depth) + TREE_INDENT : 8 }}
              title={file.from ? `${file.from} → ${file.path}` : file.path}
              onClick={() => onSelect?.(file)}
              onFocus={() => onSelect?.(file)}
              onDoubleClick={() => onOpen(file)}
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
        }}
      />
    </div>
  )
}

const relOf = (entry: { file: CommitFile }) => entry.file.rel
