/* WebStorm's "Show Diff with Working Tree": in the Log's right pane, the
   files that differ between a branch (a tag, a commit) and the IDE's
   working tree, staged or not. A double click or Enter opens one side by
   side with the working copy. */

import { EmptyState, Skeleton } from '@iii-dev/console-ui'
import { X } from 'lucide-react'
import { GitFileList } from './GitFileList'
import type { CommitFile } from './git-log-window'
import type { WorkingDiffState } from './use-git-log'

export function GitCompareFiles({
  label,
  state,
  prefix,
  top,
  narrow = false,
  onOpen,
  onClose,
}: {
  /** What the working tree is compared with: `main`, `origin/main`, `HEAD`. */
  label: string
  state: WorkingDiffState
  prefix: string
  top: string | null
  narrow?: boolean
  onOpen(file: CommitFile): void
  onClose(): void
}) {
  const { files, error, truncated } = state
  return (
    <div className="shui-git-details" data-pane="details">
      <section className="shui-git-files shui-git-compare" aria-label={`Changes between ${label} and the working tree`}>
        <div className="shui-git-compare-head">
          <p className="shui-git-pane-title">
            {label} ↔ working tree
            {files !== null ? (
              <span className="shui-git-faint">
                {' '}
                · {files.length} {files.length === 1 ? 'file' : 'files'}
                {truncated ? ', and more' : ''}
              </span>
            ) : null}
          </p>
          <button
            type="button"
            className="shui-git-toggle"
            aria-label="Close the comparison"
            title="Close"
            onClick={onClose}
          >
            <X aria-hidden />
          </button>
        </div>
        {error !== null ? (
          <p className="shui-git-note-line warn">{error}</p>
        ) : files === null ? (
          [0, 1, 2].map((row) => <Skeleton key={row} className="shui-git-skeleton" />)
        ) : files.length === 0 ? (
          <EmptyState compact title="No differences" description={`The working tree matches ${label}.`} />
        ) : (
          <GitFileList files={files} prefix={prefix} top={top} narrow={narrow} onOpen={onOpen} />
        )}
      </section>
    </div>
  )
}
