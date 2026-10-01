/* The files a commit changed, or that differ from the working tree, as a
   tree under the IDE's folder. Files outside it gather under their own
   group, their folders named from the IDE's (`../ide/src`), as in the
   Timeline. A double click or Enter opens the file's diff. */

import { useMemo } from 'react'
import { ChangeEntries } from './ChangeEntries'
import { statusLetter, statusTitle } from './git-actions'
import type { CommitFile } from './git-log-window'
import { basename } from './paths'

export function GitFileList({
  files,
  prefix,
  top,
  onOpen,
}: {
  files: readonly CommitFile[]
  /** The IDE's folder below the repository's top ('' at the top). */
  prefix: string
  /** The worktree's absolute top, when known: names the outside folders. */
  top: string | null
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
        mode="tree"
        getPath={relOf}
        outsideRoot={`${base}/${prefix.replace(/\/$/, '')}`}
        renderEntry={({ file }, depth) => (
          <button
            type="button"
            className="shui-git-file"
            style={{ paddingLeft: 8 + depth * 14 }}
            title={file.from ? `${file.from} → ${file.path}` : file.path}
            onDoubleClick={() => onOpen(file)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') onOpen(file)
            }}
          >
            <span className="shui-git-file-status" data-status={file.status} title={statusTitle(file.status)}>
              {statusLetter(file.status)}
            </span>
            <span className="shui-git-file-name">{basename(file.path)}</span>
          </button>
        )}
      />
    </div>
  )
}

const relOf = (entry: { file: CommitFile }) => entry.file.rel
