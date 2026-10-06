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
   show its history up to the commit. Its eye menu groups the files by
   folder or lists them flat, shows or hides the commit under them, and
   shows the selected file's diff there instead (WebStorm's diff preview).
   A file's context menu holds the rest: its diff in the preview tab or a
   new one, compared with the working copy (as the commit left it, or as
   it was before), its working copy or (read-only) the commit's version of
   it, the commit's change applied again or copied as a patch, and the
   working copy put back as the commit left it. */

import type { Host } from '@iii-dev/console-ui'
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
  useConfirm,
} from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import { useSplitDrag } from '@iii-dev/console-ui/hooks'
import {
  Cherry,
  ChevronsDownUp,
  ChevronsUpDown,
  ClipboardCopy,
  Copy,
  Eye,
  FileClock,
  FileDiff,
  FileDown,
  GitCompareArrows,
  History,
  Pencil,
  Undo2,
} from 'lucide-react'
import { type CSSProperties, memo, useRef, useState } from 'react'
import { type GitAction, menuItems } from './ActionRail'
import { useContextMenu } from './ContextMenu'
import { DEFAULT_DIFF_OPTIONS } from './DiffTab'
import { GitDiffPreview } from './GitDiffPreview'
import { GitFileList } from './GitFileList'
import type { CommitDetails, CommitFile } from './git-log-window'
import { basename } from './paths'
import type { CommitDetailsState } from './use-git-log'
import type { CommitFilesHow } from './use-worktree-ops'

const when = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** How the files show; kept with the Log's layout. */
export interface FilesView {
  /** Their height, set by dragging their line; null fits their list. */
  height: number | null
  /** A tree of folders; else a flat list. */
  grouped: boolean
  /** The commit's message and facts show under them. */
  info: boolean
  /** The selected file's diff shows under them, in the commit's place. */
  preview: boolean
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

/** Memoized: the Log re-renders on every keystroke in its forms and every
    page of commits; the details only when theirs changed. */
export const GitCommitDetails = memo(function GitCommitDetails({
  host,
  root,
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
  onCompare,
  onEditSource,
  onCommitFiles,
  onCopyPatch,
  onHistory,
  onOpenRevision,
  narrow = false,
}: {
  host: Host
  root: string
  state: CommitDetailsState
  selected: string | null
  /** The IDE's folder below the repository's top ('' at the top). */
  prefix: string
  /** The worktree's absolute top, when known: names the outside folders. */
  top: string | null
  /** The clone is shallow: a commit without parents may have unfetched ones. */
  shallow: boolean
  /** The commit's change to the file; `pin: false` opens the preview tab. */
  onOpenFile(file: CommitFile, details: CommitDetails, pin?: boolean): void
  onSelectCommit(sha: string): void
  view: FilesView
  onView(patch: Partial<FilesView>): void
  /** A worktree operation is running: what writes the working tree waits. */
  busy: boolean
  /** The file as it is at `ref` beside its working copy; `from` names it there. */
  onCompare(file: CommitFile, ref: string, from?: string): void
  /** The working copy, by its path below the IDE's folder. */
  onEditSource(rel: string): void
  /** Commit `sha`'s change to `paths` in the working tree (see `WorktreeOps.commitFiles`). */
  onCommitFiles(how: CommitFilesHow, sha: string, paths: string[]): void
  onCopyPatch(sha: string, paths: string[]): void
  /** The log, narrowed to `paths` and to what `sha` reaches. */
  onHistory(paths: string[], sha: string): void
  /** The file as commit `sha` left it, read-only. */
  onOpenRevision(file: CommitFile, sha: string): void
  /** Touch-sized file rows. */
  narrow?: boolean
}) {
  const { details, loading, error, branches, signature: signed } = state
  // The file picked in this commit, and how its folders were last set open.
  const [picked, setPicked] = useState<{ sha: string; path: string } | null>(null)
  const [folders, setFolders] = useState<{ sha: string; seq: number; open: boolean } | null>(null)
  // The preview's display options, kept from one commit to the next.
  const [diffOptions, setDiffOptions] = useState(DEFAULT_DIFF_OPTIONS)
  const menu = useContextMenu()
  const { confirm, dialog } = useConfirm()
  const filesRef = useRef<HTMLElement>(null)
  const shownHeight = () => filesRef.current?.getBoundingClientRect().height ?? null
  // A drag sizes the files on their section and keeps the height once, on
  // release: keeping it on every move re-rendered the whole Log.
  const dragged = useRef<number | null>(null)
  const drag = useSplitDrag<number>({
    horizontal: false,
    begin: shownHeight,
    move: (origin, delta) => {
      const files = filesRef.current
      if (files === null) return
      const height = Math.max(48, Math.round(origin + delta))
      dragged.current = height
      files.style.setProperty('--files-height', `${height}px`)
      files.setAttribute('data-sized', 'true')
    },
    step: (direction) => onView({ height: (shownHeight() ?? 0) + direction * 16 }),
  })
  const endDrag = () => {
    const height = dragged.current
    if (height === null) return
    dragged.current = null
    onView({ height })
  }
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
  const signature = signed === null ? undefined : SIGNATURES[signed]
  const committedByOther = details.committer !== details.author || details.committerEmail !== details.authorEmail
  const file = picked?.sha === details.sha ? (details.files.find((each) => each.path === picked.path) ?? null) : null
  const ours = folders?.sha === details.sha ? folders : null
  const openAll = (open: boolean) => setFolders({ sha: details.sha, seq: (ours?.seq ?? 0) + 1, open })
  const sha = details.sha
  const short = sha.slice(0, 7)
  const parent = details.parents[0] ?? null
  const busyWhy = busy ? 'another operation is running' : null
  const revert = (each: CommitFile) => onCommitFiles('revert', sha, pathsOf(each))
  const history = (each: CommitFile) => onHistory(pathsOf(each), sha)
  const getFromRevision = async (each: CommitFile) => {
    const name = basename(each.path)
    const ok = await confirm({
      title: each.status === 'deleted' ? `Remove ${name}?` : `Replace ${name} with its version at ${short}?`,
      description:
        each.status === 'deleted'
          ? `The commit deleted it: the working copy goes, uncommitted changes to it too.`
          : 'Uncommitted changes to it are lost.',
      confirmLabel: each.status === 'deleted' ? 'Remove' : 'Replace',
      tone: 'danger',
    })
    // Only its own path, as WebStorm does: a rename's old name may hold
    // another file by now, and restoring that name would delete it.
    if (ok) onCommitFiles('get', sha, [each.path])
  }
  // One list for the context menu; the bar's buttons run the same actions.
  const fileActions = (each: CommitFile): GitAction[] => [
    {
      id: 'diff',
      label: 'Show diff',
      icon: <FileDiff aria-hidden />,
      group: 'diff',
      run: () => onOpenFile(each, details, false),
    },
    {
      id: 'diff-tab',
      label: 'Show diff in a new tab',
      icon: <FileDiff aria-hidden />,
      shortcut: 'Enter',
      group: 'diff',
      run: () => onOpenFile(each, details, true),
    },
    {
      id: 'local',
      label: 'Compare with local',
      icon: <GitCompareArrows aria-hidden />,
      group: 'diff',
      blocked: each.status === 'deleted' ? 'the commit deleted it' : null,
      run: () => onCompare(each, sha),
    },
    {
      id: 'before-local',
      label: 'Compare before with local',
      icon: <GitCompareArrows aria-hidden />,
      group: 'diff',
      blocked: parent === null ? 'the commit has no parent' : each.status === 'added' ? 'the commit added it' : null,
      run: () => {
        if (parent !== null) onCompare(each, parent, each.from)
      },
    },
    {
      id: 'edit',
      label: 'Edit source',
      icon: <Pencil aria-hidden />,
      group: 'open',
      blocked: each.rel === null ? "it is outside the IDE's folder" : null,
      run: () => {
        if (each.rel !== null) onEditSource(each.rel)
      },
    },
    {
      id: 'revision',
      label: 'Open repository version',
      icon: <FileClock aria-hidden />,
      group: 'open',
      blocked: each.status === 'deleted' ? 'the commit deleted it' : null,
      run: () => onOpenRevision(each, sha),
    },
    {
      id: 'revert',
      label: 'Revert selected changes',
      icon: <Undo2 aria-hidden />,
      group: 'change',
      blocked: busyWhy,
      run: () => revert(each),
    },
    {
      id: 'cherry-pick',
      label: 'Cherry-pick selected changes',
      icon: <Cherry aria-hidden />,
      group: 'change',
      blocked: busyWhy,
      run: () => onCommitFiles('cherry-pick', sha, pathsOf(each)),
    },
    {
      id: 'patch',
      label: 'Copy as patch',
      icon: <ClipboardCopy aria-hidden />,
      group: 'change',
      run: () => onCopyPatch(sha, pathsOf(each)),
    },
    {
      id: 'get',
      label: 'Get from revision',
      icon: <FileDown aria-hidden />,
      group: 'change',
      danger: true,
      blocked: busyWhy,
      run: () => void getFromRevision(each),
    },
    {
      id: 'history',
      label: 'History up to here',
      icon: <History aria-hidden />,
      group: 'history',
      run: () => history(each),
    },
  ]
  // Under the files: the selected file's diff while the preview is on, its
  // place kept before one is picked so the files do not shrink under the
  // pointer on the first pick; else the commit.
  const below = view.preview || view.info
  return (
    <div className="shui-git-details" data-pane="details" data-info={below || undefined}>
      <section
        ref={filesRef}
        className="shui-git-files"
        aria-label={`Changed files, ${details.files.length}`}
        data-sized={(below && view.height !== null) || undefined}
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
            onClick={() => file && revert(file)}
          >
            <Undo2 aria-hidden />
          </IconButton>
          <IconButton label="History up to here" disabled={file === null} onClick={() => file && history(file)}>
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
              {/* The preview sits in the details' place: they come back with it off. */}
              <DropdownMenuCheckboxItem
                checked={view.info}
                disabled={view.preview}
                onCheckedChange={(info) => onView({ info })}
              >
                Show details
                {view.preview ? <span className="shui-git-faint"> (the diff preview is in their place)</span> : null}
              </DropdownMenuCheckboxItem>
              <DropdownMenuCheckboxItem checked={view.preview} onCheckedChange={(preview) => onView({ preview })}>
                Show diff preview
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
          narrow={narrow}
          selected={file?.path ?? null}
          onSelect={(each) => setPicked({ sha, path: each.path })}
          onMenu={(each, anchor) => {
            setPicked({ sha, path: each.path })
            menu.open(anchor, menuItems(fileActions(each)))
          }}
          onOpen={(each) => onOpenFile(each, details)}
        />
      </section>
      {below ? (
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
            onPointerUp={(event) => {
              drag.onPointerUp(event)
              endDrag()
            }}
            onPointerCancel={(event) => {
              drag.onPointerCancel(event)
              endDrag()
            }}
            onLostPointerCapture={(event) => {
              drag.onLostPointerCapture(event)
              endDrag()
            }}
          />
          {view.preview && file !== null ? (
            <GitDiffPreview
              host={host}
              root={root}
              file={file}
              sha={sha}
              parent={parent}
              options={diffOptions}
              onOptions={setDiffOptions}
            />
          ) : view.preview ? (
            <p className="shui-git-note-line">Select a file to preview its diff.</p>
          ) : (
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
                    onClick={() => void copyText(details.sha)}
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
                      {branches.total === 1
                        ? 'In branch'
                        : `In ${branches.total}${branches.partial ? '+' : ''} branches`}
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
                      <button
                        key={parent}
                        type="button"
                        className="shui-git-hash"
                        onClick={() => onSelectCommit(parent)}
                      >
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
          )}
        </>
      ) : null}
      {menu.element}
      {dialog}
    </div>
  )
})
