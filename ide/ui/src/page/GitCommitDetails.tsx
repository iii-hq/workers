/* The Log's right pane, as in WebStorm's log: the files the selected commit
   changed (against its first parent), then the commit itself. The commit
   part shows its message, hash, author and committer, its signature, the
   branches that have it, and its parents.

   A double click or Enter on a file opens its diff in an editor tab, a
   file outside the IDE's folder too (its path climbs out with `../`). A
   parent is a link to that commit in the log. */

import { Chip, EmptyState, Skeleton } from '@iii-dev/console-ui'
import { Copy } from 'lucide-react'
import { GitFileList } from './GitFileList'
import type { CommitDetails, CommitFile } from './git-log-window'
import type { CommitDetailsState } from './use-git-log'

const when = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

const SIGNATURES: Readonly<Record<string, string>> = {
  G: 'Signed',
  U: 'Signed (key not trusted)',
  X: 'Signed (signature expired)',
  Y: 'Signed (key expired)',
  R: 'Signed (key revoked)',
  B: 'Bad signature',
  E: 'Signature not checked',
}

/** "Name <email> on date", wrapping between its parts, never inside. */
function Person({ name, email, date }: { name: string; email: string; date: number }) {
  return (
    <>
      <span className="shui-git-nowrap">
        {name} <span className="shui-git-faint">&lt;{email}&gt;</span>
      </span>
      <span className="shui-git-nowrap">on {when.format(date * 1000)}</span>
    </>
  )
}

export function GitCommitDetails({
  state,
  selected,
  onOpenFile,
  onSelectCommit,
  shallow,
  prefix,
  top,
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
}) {
  const { details, loading, error, branches } = state
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
  return (
    <div className="shui-git-details" data-pane="details">
      <section className="shui-git-files" aria-label={`Changed files, ${details.files.length}`}>
        <p className="shui-git-pane-title">
          {details.files.length} {details.files.length === 1 ? 'file' : 'files'}
          {details.parents.length > 1 ? <span className="shui-git-faint"> · against the first parent</span> : null}
        </p>
        <GitFileList files={details.files} prefix={prefix} top={top} onOpen={(file) => onOpenFile(file, details)} />
      </section>
      <section className="shui-git-commit-info" aria-label="Commit">
        <p className="shui-git-info-subject">{subject}</p>
        {body !== '' ? <p className="shui-git-info-body">{body}</p> : null}
        <p className="shui-git-info-line">
          <button
            type="button"
            className="shui-git-hash"
            title="Copy the full hash"
            onClick={() => void navigator.clipboard?.writeText(details.sha)}
          >
            {details.sha.slice(0, 10)}
            <Copy aria-hidden />
          </button>
          <Person name={details.author} email={details.authorEmail} date={details.authorDate} />
        </p>
        {committedByOther ? (
          <p className="shui-git-info-line shui-git-faint">
            committed by <Person name={details.committer} email={details.committerEmail} date={details.committerDate} />
          </p>
        ) : null}
        {signature !== undefined ? (
          <p className="shui-git-info-line">
            <Chip>{signature}</Chip>
          </p>
        ) : null}
        {branches !== null && branches.total > 0 ? (
          <p className="shui-git-info-line">
            In {branches.total}
            {branches.partial ? '+' : ''} {branches.total === 1 ? 'branch' : 'branches'}:{' '}
            <span className="shui-git-faint">
              {branches.names.join(', ')}
              {branches.total > branches.names.length ? ', …' : ''}
            </span>
          </p>
        ) : null}
        {details.parents.length > 0 ? (
          <p className="shui-git-info-line">
            {details.parents.length === 1 ? 'Parent' : 'Parents'}{' '}
            {details.parents.map((parent) => (
              <button key={parent} type="button" className="shui-git-hash" onClick={() => onSelectCommit(parent)}>
                {parent.slice(0, 7)}
              </button>
            ))}
          </p>
        ) : (
          <p className="shui-git-info-line shui-git-faint">
            {shallow ? 'Its parents were not fetched (a shallow clone)' : 'The first commit'}
          </p>
        )}
        {details.truncated ? <p className="shui-git-info-line shui-git-faint">Shown in part: it is large.</p> : null}
      </section>
    </div>
  )
}
