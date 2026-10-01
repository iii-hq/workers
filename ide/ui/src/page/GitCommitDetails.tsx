/* The Log's right pane, as in WebStorm's log: the files the selected commit
   changed (against its first parent), then the commit itself. The commit
   part shows its message, hash, author and committer, its signature, the
   branches that have it, and its parents.

   A double click or Enter on a file opens its diff in an editor tab, a
   file outside the IDE's folder too (its path climbs out with `../`). A
   parent is a link to that commit in the log. The line between the files
   and the commit drags (or arrows) to size the files; a double click on it
   lets them fit their list again.

   The bar over the files acts on the selected one, as WebStorm's does:
   show its diff, revert the commit's change to it in the working tree,
   show its history in the log. Its eye menu groups the files by folder or
   lists them flat, and shows or hides the commit under them. */

import {
  Chip,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  EmptyState,
  IconButton,
  Skeleton,
} from '@iii-dev/console-ui'
import { useSplitDrag } from '@iii-dev/console-ui/hooks'
import { ChevronsDownUp, ChevronsUpDown, Copy, Eye, FileDiff, History, Undo2 } from 'lucide-react'
import { type CSSProperties, useRef, useState } from 'react'
import { GitFileList } from './GitFileList'
import type { CommitDetails, CommitFile } from './git-log-window'
import type { CommitDetailsState } from './use-git-log'

const when = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** How the files show; kept with the Log's layout. */
export interface FilesView {
  /** Their height, set by dragging their line; null fits their list. */
  height: number | null
  /** A tree of folders; else a flat list. */
  grouped: boolean
  /** The commit's message and facts show under them. */
  info: boolean
}

/** A file's paths in the commit: a rename's old one too. */
const pathsOf = (file: CommitFile) => (file.from ? [file.path, file.from] : [file.path])

const SIGNATURES: Readonly<Record<string, string>> = {
  G: 'Signed',
  U: 'Signed (key not trusted)',
  X: 'Signed (signature expired)',
  Y: 'Signed (key expired)',
  R: 'Signed (key revoked)',
  B: 'Bad signature',
  E: 'Signature not checked',
}

export function GitCommitDetails({
  state,
  selected,
  onOpenFile,
  onSelectCommit,
  shallow,
  prefix,
  top,
  view,
  onView,
  busy,
  onRevert,
  onShowHistory,
}: {
  state: CommitDetailsState
  selected: string | null
  /** The IDE's folder below the repository's top ('' at the top). */
  prefix: string
  /** The worktree's absolute top, when known: names the outside folders. */
  top: string | null
  /** The clone is shallow: a commit without parents may have unfetched ones. */
  shallow: boolean
  onOpenFile(file: CommitFile, details: CommitDetails): void
  onSelectCommit(sha: string): void
  view: FilesView
  onView(patch: Partial<FilesView>): void
  /** A worktree operation is running: a revert waits. */
  busy: boolean
  /** Undoes commit `sha`'s change to `paths` in the working tree. */
  onRevert(sha: string, paths: string[]): void
  /** The log, narrowed to `paths`. */
  onShowHistory(paths: string[]): void
}) {
  const { details, loading, error, branches } = state
  // The file picked in this commit, and how its folders were last set open.
  const [picked, setPicked] = useState<{ sha: string; path: string } | null>(null)
  const [folders, setFolders] = useState<{ sha: string; seq: number; open: boolean } | null>(null)
  const filesRef = useRef<HTMLElement>(null)
  const shownHeight = () => filesRef.current?.getBoundingClientRect().height ?? null
  const drag = useSplitDrag<number>({
    horizontal: false,
    begin: shownHeight,
    move: (origin, delta) => onView({ height: origin + delta }),
    step: (direction) => onView({ height: (shownHeight() ?? 0) + direction * 16 }),
  })
  if (selected === null) {
    return (
      <div className="shui-git-details" data-pane="details">
        <EmptyState compact title="No commit selected" description="Select a commit to see what it changed." />
      </div>
    )
  }
  if (details === null || details.sha !== selected) {
    return (
      <div className="shui-git-details" data-pane="details" aria-busy={loading || undefined}>
        {error !== null ? (
          <p className="shui-git-note-line warn">{error}</p>
        ) : (
          [0, 1, 2].map((row) => <Skeleton key={row} className="shui-git-skeleton" />)
        )}
      </div>
    )
  }
  const [subject, ...rest] = details.message.split('\n')
  const body = rest.join('\n').trim()
  const signature = SIGNATURES[details.signature]
  const committedByOther = details.committer !== details.author || details.committerEmail !== details.authorEmail
  const file = picked?.sha === details.sha ? (details.files.find((each) => each.path === picked.path) ?? null) : null
  const ours = folders?.sha === details.sha ? folders : null
  const openAll = (open: boolean) => setFolders({ sha: details.sha, seq: (ours?.seq ?? 0) + 1, open })
  return (
    <div className="shui-git-details" data-pane="details" data-info={view.info || undefined}>
      <section
        ref={filesRef}
        className="shui-git-files"
        aria-label={`Changed files, ${details.files.length}`}
        data-sized={(view.info && view.height !== null) || undefined}
        style={view.height !== null ? ({ '--files-height': `${view.height}px` } as CSSProperties) : undefined}
      >
        <div className="shui-git-files-bar" role="toolbar" aria-label="Changed files">
          <IconButton
            label="Show diff (Enter)"
            disabled={file === null}
            onClick={() => file && onOpenFile(file, details)}
          >
            <FileDiff aria-hidden />
          </IconButton>
          <IconButton
            label={busy ? 'Revert selected changes: another operation is running' : 'Revert selected changes'}
            disabled={file === null || busy}
            onClick={() => file && onRevert(details.sha, pathsOf(file))}
          >
            <Undo2 aria-hidden />
          </IconButton>
          <IconButton
            label="Show history"
            disabled={file === null}
            onClick={() => file && onShowHistory(pathsOf(file))}
          >
            <History aria-hidden />
          </IconButton>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <IconButton label="View options">
                <Eye aria-hidden />
              </IconButton>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="shui-git-view-menu">
              <DropdownMenuLabel>Group by</DropdownMenuLabel>
              <DropdownMenuCheckboxItem checked={view.grouped} onCheckedChange={(grouped) => onView({ grouped })}>
                Directory
              </DropdownMenuCheckboxItem>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>Layout</DropdownMenuLabel>
              <DropdownMenuCheckboxItem checked={view.info} onCheckedChange={(info) => onView({ info })}>
                Show details
              </DropdownMenuCheckboxItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <p className="shui-git-pane-title">
            {details.files.length} {details.files.length === 1 ? 'file' : 'files'}
            {details.parents.length > 1 ? <span className="shui-git-faint"> · against the first parent</span> : null}
          </p>
          <IconButton label="Expand all folders" disabled={!view.grouped} onClick={() => openAll(true)}>
            <ChevronsUpDown aria-hidden />
          </IconButton>
          <IconButton label="Collapse all folders" disabled={!view.grouped} onClick={() => openAll(false)}>
            <ChevronsDownUp aria-hidden />
          </IconButton>
        </div>
        <GitFileList
          key={ours?.seq ?? 0}
          files={details.files}
          prefix={prefix}
          top={top}
          grouped={view.grouped}
          open={ours?.open ?? true}
          selected={file?.path ?? null}
          onSelect={(each) => setPicked({ sha: details.sha, path: each.path })}
          onOpen={(each) => onOpenFile(each, details)}
        />
      </section>
      {view.info ? (
        <>
          {/* biome-ignore lint/a11y/useSemanticElements: an interactive range separator, not a thematic break */}
          <div
            role="separator"
            tabIndex={0}
            className="shui-git-hsash"
            aria-label="Resize the changed files"
            aria-orientation="horizontal"
            aria-valuemin={48}
            aria-valuenow={view.height ?? undefined}
            title="Drag to resize; double-click to fit the files"
            onDoubleClick={() => onView({ height: null })}
            {...drag}
          />
          <section className="shui-git-commit-info" aria-label="Commit">
            <p className="shui-git-info-subject">{subject}</p>
            {body !== '' ? <p className="shui-git-info-body">{body}</p> : null}
            {/* One label and one value a row, so nothing wraps into the next line's place. */}
            <dl className="shui-git-kv">
              <dt>Hash</dt>
              <dd>
                <button
                  type="button"
                  className="shui-git-hash"
                  title="Copy the full hash"
                  onClick={() => void navigator.clipboard?.writeText(details.sha)}
                >
                  {details.sha.slice(0, 10)}
                  <Copy aria-hidden />
                </button>
              </dd>
              <dt>Author</dt>
              <dd>
                {details.author} <span className="shui-git-faint">{details.authorEmail}</span>
              </dd>
              <dt>Date</dt>
              <dd>{when.format(details.authorDate * 1000)}</dd>
              {committedByOther ? (
                <>
                  <dt>Committed by</dt>
                  <dd>
                    {details.committer}{' '}
                    <span className="shui-git-faint">{when.format(details.committerDate * 1000)}</span>
                  </dd>
                </>
              ) : null}
              {signature !== undefined ? (
                <>
                  <dt>Signature</dt>
                  <dd>
                    <Chip>{signature}</Chip>
                  </dd>
                </>
              ) : null}
              {branches !== null && branches.total > 0 ? (
                <>
                  <dt>
                    {branches.total === 1 ? 'In branch' : `In ${branches.total}${branches.partial ? '+' : ''} branches`}
                  </dt>
                  <dd>
                    {branches.names.join(', ')}
                    {branches.total > branches.names.length ? ', …' : ''}
                  </dd>
                </>
              ) : null}
              <dt>{details.parents.length > 1 ? 'Parents' : 'Parent'}</dt>
              <dd>
                {details.parents.length > 0 ? (
                  details.parents.map((parent) => (
                    <button key={parent} type="button" className="shui-git-hash" onClick={() => onSelectCommit(parent)}>
                      {parent.slice(0, 7)}
                    </button>
                  ))
                ) : (
                  <span className="shui-git-faint">
                    {shallow ? 'not fetched (a shallow clone)' : 'none: the first commit'}
                  </span>
                )}
              </dd>
            </dl>
            {details.truncated ? (
              <p className="shui-git-info-line shui-git-faint">Shown in part: it is large.</p>
            ) : null}
          </section>
        </>
      ) : null}
    </div>
  )
}
