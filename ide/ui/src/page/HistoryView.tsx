/* The History tab (IntelliJ's Log): the current branch's first-parent
   history as one lane, refs as chips, a filter over message, author and hash,
   and the selected commit's message and files (each opens its diff against
   the parent), with Revert… and New branch…. */

import type { Host } from '@iii-dev/console-ui'
import { Button, ConfirmDialog, EmptyState, IconButton, SearchField, Skeleton, StatusPanel } from '@iii-dev/console-ui'
import { copyText, errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { CircleAlert, Copy, GitBranch, RefreshCw, Tag } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { DiffSource } from './diff-source'
import { gitCreateBranch, gitRevert } from './git-actions'
import { type GitCommitDetails, type GitLogCommit, gitCommitDetails, gitLog, revisionParent } from './git-log'
import { FileList, type Load } from './StashView'
import { TextDialog } from './TextDialog'

interface HistoryViewProps {
  host: Host
  root: string | null
  refreshEpoch: number
  activeSource: DiffSource | null
  activePath: string | null
  onOpenDiff: (path: string, source: DiffSource, pin: boolean) => void
  onChanged: () => void
}

/** Case-insensitive match over subject, author and either hash. */
export function matchesCommit(commit: Pick<GitLogCommit, 'subject' | 'author' | 'sha'>, query: string): boolean {
  const needle = query.trim().toLowerCase()
  if (needle === '') return true
  return `${commit.subject}\n${commit.author}\n${commit.sha}`.toLowerCase().includes(needle)
}

export function HistoryView({
  host,
  root,
  refreshEpoch,
  activeSource,
  activePath,
  onOpenDiff,
  onChanged,
}: HistoryViewProps) {
  const [log, setLog] = useState<Load<GitLogCommit[]>>({ kind: 'loading' })
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const [details, setDetails] = useState<Load<GitCommitDetails> | null>(null)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<{ text: string; failed: boolean } | null>(null)
  const [revertFor, setRevertFor] = useState<GitLogCommit | null>(null)
  const [branchFor, setBranchFor] = useState<GitLogCommit | null>(null)
  const [epoch, setEpoch] = useState(0)
  const seqRef = useRef(0)

  useEffect(() => {
    if (root === null) return
    const seq = ++seqRef.current
    gitLog(host, root)
      .then((commits) => {
        if (seqRef.current !== seq) return
        setLog({ kind: 'ready', value: commits })
        setSelected((current) =>
          commits.some((commit) => commit.sha === current) ? current : (commits[0]?.sha ?? null),
        )
      })
      .catch((err: unknown) => seqRef.current === seq && setLog({ kind: 'error', message: errorMessage(err) }))
  }, [host, root, refreshEpoch, epoch])

  const commits = log.kind === 'ready' ? log.value : []
  const shown = useMemo(() => commits.filter((commit) => matchesCommit(commit, query)), [commits, query])
  const current = commits.find((commit) => commit.sha === selected) ?? null

  useEffect(() => {
    if (root === null || current === null) {
      setDetails(null)
      return
    }
    let live = true
    setDetails({ kind: 'loading' })
    gitCommitDetails(host, root, current)
      .then((value) => live && setDetails({ kind: 'ready', value }))
      .catch((err: unknown) => live && setDetails({ kind: 'error', message: errorMessage(err) }))
    return () => {
      live = false
    }
  }, [host, root, current])

  const perform = useCallback(
    async (label: string, action: () => Promise<string>) => {
      setBusy(true)
      setNote(null)
      try {
        setNote({ text: await action(), failed: false })
      } catch (err: unknown) {
        setNote({ text: `${label} failed: ${errorMessage(err)}`, failed: true })
      } finally {
        setBusy(false)
        setEpoch((value) => value + 1)
        onChanged()
      }
    },
    [onChanged],
  )

  const source = (commit: GitLogCommit): DiffSource => ({
    type: 'revision',
    from: revisionParent(commit),
    to: commit.sha,
    label: commit.short,
  })
  const body = details?.kind === 'ready' ? details.value.message.split('\n').slice(1).join('\n').trim() : ''

  return (
    <div className="shui-history">
      <div className="shui-commit-toolbar" role="toolbar" aria-label="History">
        <SearchField
          className="shui-history-filter"
          value={query}
          onChange={setQuery}
          placeholder="Message, author or hash"
          aria-label="Filter commits"
        />
        <IconButton label="Refresh" disabled={busy} onClick={() => setEpoch((value) => value + 1)}>
          <RefreshCw aria-hidden />
        </IconButton>
      </div>

      {note ? (
        <div className="shui-commit-note pad" data-failed={note.failed || undefined} role="status">
          <span>{note.text}</span>
        </div>
      ) : null}

      <div className="shui-stash-list">
        {log.kind === 'loading' ? (
          <div className="shui-commit-skeleton">
            <Skeleton className="shui-commit-skeleton-row tall" />
            <Skeleton className="shui-commit-skeleton-row tall" />
            <Skeleton className="shui-commit-skeleton-row tall" />
          </div>
        ) : log.kind === 'error' ? (
          <StatusPanel
            variant="alert"
            icon={<CircleAlert aria-hidden />}
            headline="Couldn't read the history"
            detail={log.message}
            action={
              <Button type="button" variant="pill" size="sm" onClick={() => setEpoch((value) => value + 1)}>
                Retry
              </Button>
            }
          />
        ) : commits.length === 0 ? (
          <EmptyState compact title="No commits yet" description="The first commit made here shows up in this list." />
        ) : shown.length === 0 ? (
          <p className="shui-log-details-empty pad">No commits match “{query.trim()}”.</p>
        ) : (
          <ul className="shui-log-rows" aria-label="Commits">
            {shown.map((commit, index) => {
              const head = commit.refs.some((ref) => ref.kind === 'head')
              return (
                <li key={commit.sha}>
                  <div className="shui-log-row graph" data-selected={commit.sha === selected || undefined}>
                    <span
                      className="shui-log-lane"
                      data-first={(index === 0 && query.trim() === '') || undefined}
                      data-head={head || undefined}
                      data-merge={commit.parents.length > 1 || undefined}
                      aria-hidden
                    >
                      <span className="dot" />
                    </span>
                    <button
                      type="button"
                      className="shui-log-main"
                      aria-current={commit.sha === selected || undefined}
                      title={commit.subject}
                      onClick={() => setSelected(commit.sha)}
                    >
                      <span className="subject">{commit.subject}</span>
                      <span className="meta">
                        {commit.refs.length > 0 ? (
                          <span
                            className="shui-log-ref"
                            data-kind={commit.refs[0].kind}
                            title={commit.refs.map((ref) => ref.name).join(', ')}
                          >
                            {commit.refs[0].kind === 'tag' ? <Tag aria-hidden /> : <GitBranch aria-hidden />}
                            <span className="shui-log-ref-label">{commit.refs[0].name}</span>
                          </span>
                        ) : null}
                        {commit.refs.length > 1 ? (
                          <span className="shui-log-more">+{commit.refs.length - 1}</span>
                        ) : null}
                        <span className="mono">{commit.short}</span>
                        <span className="author">{commit.author}</span>
                        <span className="spacer" />
                        <span className="num" title={new Date(commit.time * 1000).toLocaleString()}>
                          {formatRelative(commit.time)}
                        </span>
                      </span>
                    </button>
                  </div>
                </li>
              )
            })}
          </ul>
        )}
      </div>

      {current ? (
        <section className="shui-log-details" aria-label={`Commit ${current.short}`}>
          <div className="shui-log-details-head">
            <span className="mono">{current.short}</span>
            <IconButton label="Copy the commit hash" onClick={() => void copyText(current.sha)}>
              <Copy aria-hidden />
            </IconButton>
            <span className="spacer" />
            <span className="num faint" title={new Date(current.time * 1000).toLocaleString()}>
              {new Date(current.time * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })}
            </span>
          </div>
          <p className="shui-log-details-subject">{current.subject}</p>
          <span
            className="shui-log-details-author"
            title={details?.kind === 'ready' && details.value.email ? details.value.email : undefined}
          >
            {current.author}
          </span>
          {body ? <p className="shui-log-details-body">{body}</p> : null}
          <FileList
            files={
              details === null
                ? null
                : details.kind === 'ready'
                  ? { kind: 'ready', value: details.value.files }
                  : details
            }
            isActive={(file) =>
              activePath === file.path && activeSource?.type === 'revision' && activeSource.to === current.sha
            }
            onOpen={(file, pin) => onOpenDiff(file.path, source(current), pin)}
          />
          <div className="shui-log-details-actions">
            <Button type="button" variant="pill" size="sm" disabled={busy} onClick={() => setRevertFor(current)}>
              Revert…
            </Button>
            <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={() => setBranchFor(current)}>
              New branch…
            </Button>
          </div>
        </section>
      ) : null}

      <ConfirmDialog
        open={revertFor !== null}
        onOpenChange={(open) => (open ? undefined : setRevertFor(null))}
        title={revertFor ? `Revert ${revertFor.short}?` : 'Revert commit?'}
        description="Makes a new commit on the current branch that undoes this one."
        details={revertFor ? [revertFor.subject] : undefined}
        confirmLabel="Revert"
        onCancel={() => setRevertFor(null)}
        onConfirm={() => {
          const commit = revertFor
          setRevertFor(null)
          if (!commit) return
          void perform('revert', async () => {
            await gitRevert(host, root ?? '', commit.sha)
            return `reverted ${commit.short}`
          })
        }}
      />
      <TextDialog
        open={branchFor !== null}
        title="New branch"
        description={branchFor ? `Creates a branch at ${branchFor.short} without switching to it.` : undefined}
        label="Branch name"
        placeholder="feat/…"
        confirmLabel="Create branch"
        onCancel={() => setBranchFor(null)}
        onConfirm={(name) => {
          const commit = branchFor
          setBranchFor(null)
          if (!commit) return
          void perform('branch', async () => {
            await gitCreateBranch(host, root ?? '', name, commit.sha)
            return `created ${name} at ${commit.short}`
          })
        }}
      />
    </div>
  )
}
