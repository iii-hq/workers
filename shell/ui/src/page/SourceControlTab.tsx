/* The Source Control view — VS Code's SCM panel over the browsed root:
   a commit box, then "Staged Changes" and "Changes" with the letter and
   colour VS Code uses per status and the same hover verbs (open, discard,
   stage / unstage). Clicking a row opens that file's diff for its side —
   index against HEAD for staged rows, worktree against index otherwise. */

import { ConfirmDialog, IconButton } from '@iii-dev/console-ui'
import { Check, GitBranch, Minus, Plus, RefreshCw, Undo2 } from 'lucide-react'
import { useState } from 'react'
import { FileTypeIcon } from './file-type-icon'
import type { GitComparisonEntry } from './git'
import { statusLetter, statusTitle } from './git-actions'
import { basename, dirname } from './paths'
import type { SourceControlState } from './use-source-control'
import { ViewHeader, ViewSection } from './ViewHeader'

interface SourceControlTabProps {
  scm: SourceControlState
  /** The diff tab in front, when it shows one of these rows. */
  activePath: string | null
  activeSide: 'staged' | 'unstaged' | null
  /** Click opens the row's diff as a preview tab; double click keeps it. */
  onOpenChange: (scope: 'staged' | 'unstaged', path: string, pin: boolean) => void
  onOpenFile: (path: string) => void
}

type PendingDiscard = { kind: 'one'; entry: GitComparisonEntry } | { kind: 'all'; entries: readonly GitComparisonEntry[] }

export function SourceControlTab({ scm, activePath, activeSide, onOpenChange, onOpenFile }: SourceControlTabProps) {
  const [message, setMessage] = useState('')
  const [stagedOpen, setStagedOpen] = useState(true)
  const [changesOpen, setChangesOpen] = useState(true)
  const [pending, setPending] = useState<PendingDiscard | null>(null)

  // A branch switch starts a fresh message (adjusted during render, no
  // extra pass).
  const [branchSeen, setBranchSeen] = useState(scm.branch)
  if (branchSeen !== scm.branch) {
    setBranchSeen(scm.branch)
    setMessage('')
  }

  const submitCommit = async () => {
    if (scm.busy || message.trim() === '') return
    if (await scm.commit(message)) setMessage('')
  }

  const pendingCount = pending === null ? 0 : pending.kind === 'one' ? 1 : pending.entries.length

  return (
    <div className="shui-scm">
      <ViewHeader
        title="Source control"
        detail={
          scm.branch ? (
            <span className="shui-scm-branch" title={`On branch ${scm.branch}`}>
              <GitBranch aria-hidden />
              {scm.branch}
            </span>
          ) : undefined
        }
        actions={
          <IconButton label="Refresh" onClick={scm.reload} disabled={scm.busy}>
            <RefreshCw aria-hidden />
          </IconButton>
        }
      />
      {scm.phase === 'not-a-repo' ? (
        <div className="shui-side-note">This folder is not inside a Git repository.</div>
      ) : scm.phase === 'error' ? (
        <div className="shui-side-note warn">{scm.error}</div>
      ) : scm.phase === 'loading' || scm.phase === 'idle' ? (
        <div className="shui-side-note">reading git status…</div>
      ) : (
        <>
          <form
            className="shui-scm-commit"
            onSubmit={(event) => {
              event.preventDefault()
              void submitCommit()
            }}
          >
            <textarea
              className="shui-scm-message"
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              placeholder={`Message (${navigator.platform.includes('Mac') ? '⌘' : 'Ctrl+'}Enter to commit${scm.branch ? ` on "${scm.branch}"` : ''})`}
              rows={2}
              spellCheck
              onKeyDown={(event) => {
                if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
                  event.preventDefault()
                  void submitCommit()
                }
              }}
            />
            <button
              type="submit"
              className="shui-scm-commit-btn"
              disabled={scm.busy || message.trim() === '' || scm.staged.length === 0}
              title={scm.staged.length === 0 ? 'stage changes first' : 'commit staged changes'}
            >
              <Check aria-hidden />
              Commit
            </button>
          </form>
          {scm.note ? (
            <div className={`shui-scm-note${scm.note.includes('failed') ? ' warn' : ''}`} role="status">
              {scm.note}
            </div>
          ) : null}
          <div className="shui-scm-sections">
            {scm.staged.length > 0 ? (
              <ViewSection
                title="Staged changes"
                count={scm.staged.length}
                open={stagedOpen}
                onToggle={() => setStagedOpen((value) => !value)}
                actions={
                  <IconButton label="Unstage all changes" disabled={scm.busy} onClick={() => void scm.unstageAll()}>
                    <Minus aria-hidden />
                  </IconButton>
                }
              >
                {scm.staged.map((entry) => (
                  <ChangeRow
                    key={`staged:${entry.path}`}
                    entry={entry}
                    active={activeSide === 'staged' && activePath === entry.path}
                    busy={scm.busy}
                    onOpen={(pin) => onOpenChange('staged', entry.path, pin)}
                    onOpenFile={entry.status === 'deleted' ? undefined : () => onOpenFile(entry.path)}
                    actions={
                      <IconButton label="Unstage changes" disabled={scm.busy} onClick={() => void scm.unstage([entry.path])}>
                        <Minus aria-hidden />
                      </IconButton>
                    }
                  />
                ))}
              </ViewSection>
            ) : null}
            <ViewSection
              title="Changes"
              count={scm.unstaged.length}
              open={changesOpen}
              onToggle={() => setChangesOpen((value) => !value)}
              actions={
                <>
                  <IconButton
                    label="Discard all changes"
                    disabled={scm.busy || scm.unstaged.length === 0}
                    onClick={() => setPending({ kind: 'all', entries: scm.unstaged })}
                  >
                    <Undo2 aria-hidden />
                  </IconButton>
                  <IconButton label="Stage all changes" disabled={scm.busy || scm.unstaged.length === 0} onClick={() => void scm.stageAll()}>
                    <Plus aria-hidden />
                  </IconButton>
                </>
              }
            >
              {scm.unstaged.length === 0 ? (
                <div className="shui-scm-empty">{scm.staged.length === 0 ? 'No changes' : 'No unstaged changes'}</div>
              ) : (
                scm.unstaged.map((entry) => (
                  <ChangeRow
                    key={`unstaged:${entry.path}`}
                    entry={entry}
                    active={activeSide === 'unstaged' && activePath === entry.path}
                    busy={scm.busy}
                    onOpen={(pin) => onOpenChange('unstaged', entry.path, pin)}
                    onOpenFile={entry.status === 'deleted' ? undefined : () => onOpenFile(entry.path)}
                    actions={
                      <>
                        <IconButton label="Discard changes" disabled={scm.busy} onClick={() => setPending({ kind: 'one', entry })}>
                          <Undo2 aria-hidden />
                        </IconButton>
                        <IconButton label="Stage changes" disabled={scm.busy} onClick={() => void scm.stage([entry.path])}>
                          <Plus aria-hidden />
                        </IconButton>
                      </>
                    }
                  />
                ))
              )}
            </ViewSection>
          </div>
        </>
      )}
      <ConfirmDialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) setPending(null)
        }}
        title={pending?.kind === 'one' ? `Discard changes in ${basename(pending.entry.path)}?` : `Discard all ${pendingCount} changes?`}
        description="Working-tree changes are lost; untracked files are deleted. This cannot be undone."
        details={pending === null ? undefined : pending.kind === 'one' ? [pending.entry.path] : pending.entries.slice(0, 8).map((entry) => entry.path)}
        confirmLabel="Discard"
        onConfirm={() => {
          const target = pending
          setPending(null)
          if (!target) return
          void scm.discard(target.kind === 'one' ? [target.entry] : target.entries)
        }}
        onCancel={() => setPending(null)}
      />
    </div>
  )
}

function ChangeRow({
  entry,
  active,
  busy,
  onOpen,
  onOpenFile,
  actions,
}: {
  entry: GitComparisonEntry
  active: boolean
  busy: boolean
  onOpen: (pin: boolean) => void
  onOpenFile?: () => void
  actions: React.ReactNode
}) {
  const name = basename(entry.path)
  const dir = dirname(entry.path)
  return (
    <div className={`shui-scm-row${active ? ' active' : ''}`} data-status={entry.status}>
      <button
        type="button"
        className="shui-scm-row-main"
        onClick={() => onOpen(false)}
        onDoubleClick={() => onOpen(true)}
        title={entry.path}
        disabled={busy}
      >
        <FileTypeIcon path={entry.path} className="file-icon" />
        <span className="name">{name}</span>
        {dir ? <span className="dir">{dir}</span> : null}
        {entry.renameFrom ? <span className="dir">from {entry.renameFrom}</span> : null}
      </button>
      <span className="shui-scm-row-actions">
        {onOpenFile ? (
          <IconButton label="Open the file" disabled={busy} onClick={onOpenFile}>
            <FileTypeIcon path={entry.path} className="file-icon" />
          </IconButton>
        ) : null}
        {actions}
      </span>
      <span className="shui-scm-status" title={statusTitle(entry.status)}>
        {statusLetter(entry.status)}
      </span>
    </div>
  )
}
