/* The Timeline view: every Harness turn of the chat this page follows,
   newest first, each a folder-like group named after the message that
   started it, holding the files the turn changed. Changes made by
   sub-agents the turn spawned are already folded into it by the worker;
   a row tells which agent did the typing. Clicking a file opens that
   turn's diff of it as a tab; a turn or a single file can be rolled back
   from the worker's pre-image store. */

import { ConfirmDialog, IconButton } from '@iii-dev/console-ui'
import { Bot, ChevronDown, ChevronRight, RefreshCw, Undo2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { FileTypeIcon } from './file-type-icon'
import { basename, dirname } from './paths'
import { relativeToRoot, type SessionTurnSummary, turnLabel, turnTitle } from './turns'
import { ViewHeader } from './ViewHeader'

interface TimelineTabProps {
  turns: readonly SessionTurnSummary[]
  root: string
  hasSession: boolean
  runningTurnId: string | null
  /** The diff tab in front: which turn and file it shows, if any. */
  activeTurnId: string | null
  activePath: string | null
  reverting: string | null
  note: string | null
  onRefresh: () => void
  onOpenFile: (turnId: string, relPath: string, pin: boolean) => void
  onOpenWorkingFile: (relPath: string) => void
  onRevertTurn: (turnId: string) => void
  onRevertFile: (turnId: string, absPath: string) => void
}

function kindLetter(kind: string): string {
  switch (kind) {
    case 'created':
      return 'A'
    case 'deleted':
      return 'D'
    case 'moved':
      return 'R'
    default:
      return 'M'
  }
}

function kindStatus(kind: string): string {
  switch (kind) {
    case 'created':
      return 'added'
    case 'deleted':
      return 'deleted'
    case 'moved':
      return 'renamed'
    default:
      return 'modified'
  }
}

type PendingRevert =
  | { kind: 'turn'; turnId: string; title: string; count: number }
  | { kind: 'file'; turnId: string; path: string }

export function TimelineTab({
  turns,
  root,
  hasSession,
  runningTurnId,
  activeTurnId,
  activePath,
  reverting,
  note,
  onRefresh,
  onOpenFile,
  onOpenWorkingFile,
  onRevertTurn,
  onRevertFile,
}: TimelineTabProps) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set())
  const [pending, setPending] = useState<PendingRevert | null>(null)

  // Groups open by default; the ones the user closed stay closed. A turn
  // that starts running while an older one was collapsed still opens.
  const newest = turns[0]?.turn_id ?? null
  useEffect(() => {
    if (newest === null) return
    setCollapsed((previous) => {
      if (!previous.has(newest)) return previous
      const next = new Set(previous)
      next.delete(newest)
      return next
    })
  }, [newest])

  const toggle = (turnId: string) =>
    setCollapsed((previous) => {
      const next = new Set(previous)
      if (next.has(turnId)) next.delete(turnId)
      else next.add(turnId)
      return next
    })

  return (
    <div className="shui-timeline">
      <ViewHeader
        title="Timeline"
        detail={turns.length > 0 ? `${turns.length} ${turns.length === 1 ? 'turn' : 'turns'}` : undefined}
        actions={
          <IconButton label="Refresh timeline" onClick={onRefresh}>
            <RefreshCw aria-hidden />
          </IconButton>
        }
      />
      {!hasSession ? (
        <div className="shui-side-note">Open this page beside a chat to follow its turns.</div>
      ) : turns.length === 0 ? (
        <div className="shui-side-note">No turn has changed files yet.</div>
      ) : (
        <div className="shui-timeline-list">
          {turns.map((turn, index) => {
            const open = !collapsed.has(turn.turn_id)
            const running = runningTurnId === turn.turn_id || turn.ended_at == null
            const ordinal = turns.length - index
            const title = turnTitle(turn, ordinal)
            const meta = turnLabel(turn)
            const activeTurn = activeTurnId === turn.turn_id
            return (
              <section
                key={turn.turn_id}
                className={`shui-timeline-turn${activeTurn ? ' active' : ''}${running ? ' running' : ''}${open ? ' open' : ''}`}
              >
                <div className="shui-timeline-turn-head">
                  <button
                    type="button"
                    className="shui-timeline-turn-main"
                    aria-expanded={open}
                    onClick={() => toggle(turn.turn_id)}
                    title={`${title}\n${meta}\n${turn.turn_id}`}
                  >
                    {open ? <ChevronDown aria-hidden className="chevron" /> : <ChevronRight aria-hidden className="chevron" />}
                    <span className="label">{title}</span>
                    <span className="meta">{meta}</span>
                    {running ? <span className="pill">running</span> : null}
                  </button>
                  <span className="shui-view-actions">
                    <IconButton
                      label="Revert this turn"
                      disabled={reverting !== null || running || turn.file_count === 0}
                      onClick={() => setPending({ kind: 'turn', turnId: turn.turn_id, title, count: turn.file_count })}
                    >
                      <Undo2 aria-hidden />
                    </IconButton>
                  </span>
                </div>
                {open ? (
                  <div className="shui-timeline-files">
                    {turn.files.length === 0 ? (
                      <div className="shui-scm-empty">{running ? 'no file changes yet' : 'no file changes'}</div>
                    ) : (
                      turn.files.map((file) => {
                        const rel = relativeToRoot(file.path, root)
                        const shown = rel ?? file.path
                        const agentName = file.agent ? (file.agent.name ?? 'sub-agent') : null
                        const isActive = activeTurn && rel !== null && activePath === rel
                        return (
                          <div
                            key={file.path}
                            className={`shui-scm-row${isActive ? ' active' : ''}${rel === null ? ' outside' : ''}`}
                            data-status={kindStatus(file.kind)}
                          >
                            <button
                              type="button"
                              className="shui-scm-row-main"
                              disabled={rel === null}
                              title={
                                rel === null
                                  ? `${file.path} (outside this folder)`
                                  : agentName
                                    ? `${file.path}\nchanged by ${agentName}`
                                    : file.path
                              }
                              onClick={() => {
                                if (rel !== null) onOpenFile(turn.turn_id, rel, false)
                              }}
                              onDoubleClick={() => {
                                if (rel !== null) onOpenFile(turn.turn_id, rel, true)
                              }}
                            >
                              <FileTypeIcon path={shown} className="file-icon" />
                              <span className="name">{basename(shown)}</span>
                              {dirname(shown) ? <span className="dir">{dirname(shown)}</span> : null}
                              {agentName ? (
                                <span className="shui-agent-tag" title={`changed by ${agentName}`}>
                                  <Bot aria-hidden />
                                  {agentName}
                                </span>
                              ) : null}
                            </button>
                            <span className="shui-scm-row-actions">
                              {rel !== null && file.kind !== 'deleted' ? (
                                <IconButton label="Open the file" onClick={() => onOpenWorkingFile(rel)}>
                                  <FileTypeIcon path={shown} className="file-icon" />
                                </IconButton>
                              ) : null}
                              <IconButton
                                label="Revert this file"
                                disabled={reverting !== null || running}
                                onClick={() => setPending({ kind: 'file', turnId: turn.turn_id, path: file.path })}
                              >
                                <Undo2 aria-hidden />
                              </IconButton>
                            </span>
                            <span className="shui-scm-status" title={kindStatus(file.kind)}>
                              {kindLetter(file.kind)}
                            </span>
                          </div>
                        )
                      })
                    )}
                  </div>
                ) : null}
              </section>
            )
          })}
        </div>
      )}
      {note ? (
        <div className={`shui-scm-note${note.includes('could not') || note.includes('failed') ? ' warn' : ''}`} role="status">
          {note}
        </div>
      ) : null}
      <ConfirmDialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) setPending(null)
        }}
        title={pending?.kind === 'turn' ? `Revert "${pending.title}"?` : `Revert ${pending ? basename(pending.path) : ''}?`}
        description={
          pending?.kind === 'turn'
            ? `Every file this turn changed goes back to how it was before the turn: ${pending.count} ${pending.count === 1 ? 'file' : 'files'}. Edits made after the turn to the same files are lost.`
            : 'The file goes back to how it was before this turn. Later edits to it are lost.'
        }
        details={pending?.kind === 'file' ? [pending.path] : undefined}
        confirmLabel="Revert"
        onConfirm={() => {
          const target = pending
          setPending(null)
          if (!target) return
          if (target.kind === 'turn') onRevertTurn(target.turnId)
          else onRevertFile(target.turnId, target.path)
        }}
        onCancel={() => setPending(null)}
      />
    </div>
  )
}
