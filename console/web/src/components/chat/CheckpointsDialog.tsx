import { useCallback, useEffect, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import {
  type CheckpointGroup,
  groupCheckpoints,
  listCheckpoints,
  type UndoRecord,
  undoCheckpoint,
} from '@/lib/backend/coder-checkpoints'

interface CheckpointsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The conversation's session workspace — journal root for `coder::*`. */
  workingDir?: string | null
  /** Filters the shared per-root journal down to this conversation's records. */
  sessionId: string
  /** Warns that undoing while a turn runs may fight the agent's writes. */
  sessionBusy?: boolean
}

type LoadState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; groups: CheckpointGroup[]; truncated: boolean }

const basename = (p: string): string => p.split('/').pop() || p

/** Time for today's records; date + time once a record is from another day. */
export function formatRecordTime(ts: number, now = new Date()): string {
  const d = new Date(ts)
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  const time = d.toLocaleTimeString()
  if (sameDay) return time
  const date = d.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  })
  return `${date} ${time}`
}

function formatUndoSummary(undone: UndoRecord[], wasRevert: boolean): string {
  if (undone.length === 0) return 'nothing to undo.'
  const restored = undone.reduce((n, u) => n + u.restored.length, 0)
  const removed = undone.reduce((n, u) => n + u.removed.length, 0)
  const skipped = undone.reduce((n, u) => n + u.skipped.length, 0)
  const parts = [`${restored} restored`, `${removed} removed`]
  if (skipped > 0) parts.push(`${skipped} skipped`)
  return `${wasRevert ? 'redone' : 'undone'} — ${parts.join(', ')}.`
}

/**
 * File-history / undo surface: lists the shell's journal records (newest-first,
 * grouped per turn) and reverses a group with one `coder::undo` call. Opened
 * from the ChatView working-dir footer row; mirrors FilesystemAccessDialog.
 */
export function CheckpointsDialog({
  open,
  onOpenChange,
  workingDir,
  sessionId,
  sessionBusy,
}: CheckpointsDialogProps) {
  const [state, setState] = useState<LoadState>({ status: 'idle' })
  const [undoingKey, setUndoingKey] = useState<string | null>(null)
  const [summary, setSummary] = useState<string | null>(null)

  const load = useCallback(async () => {
    if (!workingDir) return
    setState({ status: 'loading' })
    try {
      const res = await listCheckpoints(workingDir)
      // The journal is per-root and shared across every session using this
      // directory; only this conversation's records belong in the dialog.
      const mine = res.records.filter((r) => r.sessionId === sessionId)
      setState({
        status: 'ready',
        groups: groupCheckpoints(mine),
        truncated: res.truncated,
      })
    } catch (err) {
      setState({
        status: 'error',
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }, [workingDir, sessionId])

  useEffect(() => {
    if (!open) return
    setSummary(null)
    void load()
  }, [open, load])

  const handleUndo = useCallback(
    async (group: CheckpointGroup) => {
      if (!workingDir) return
      setUndoingKey(group.key)
      setSummary(null)
      try {
        if (!group.turnId) return
        const undone = await undoCheckpoint(workingDir, {
          turnId: group.turnId,
          sessionId,
        })
        setSummary(formatUndoSummary(undone, group.isRevert))
        await load()
      } catch (err) {
        setSummary(
          `undo failed — ${err instanceof Error ? err.message : String(err)}`,
        )
      } finally {
        setUndoingKey(null)
      }
    },
    [workingDir, sessionId, load],
  )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogTitle className="text-[14px]">checkpoints</DialogTitle>
        <DialogDescription className="mt-1">
          undo file changes the agent made in this conversation.
        </DialogDescription>

        {sessionBusy ? (
          <p className="mt-3 font-mono text-[11px] text-ink-faint">
            the agent is running — undoing files it's editing may make it redo
            work.
          </p>
        ) : null}

        {summary ? (
          <div className="mt-3 border border-rule-2 bg-bg px-2 py-1.5 font-mono text-[11px] text-ink">
            {summary}
          </div>
        ) : null}

        <div className="mt-4 flex flex-col gap-2">
          {renderBody(state, {
            workingDir,
            undoingKey,
            onUndo: handleUndo,
            onRetry: load,
          })}
        </div>
      </DialogContent>
    </Dialog>
  )
}

interface BodyHandlers {
  workingDir?: string | null
  undoingKey: string | null
  onUndo: (group: CheckpointGroup) => void
  onRetry: () => void
}

// Exported for unit tests (no DOM harness in this project — tests inspect the
// returned element tree, mirroring the state-view tests).
export function renderBody(state: LoadState, h: BodyHandlers) {
  if (!h.workingDir) return <Empty text="set a working directory first." />
  if (state.status === 'loading' || state.status === 'idle')
    return <Empty text="loading…" />
  if (state.status === 'error')
    return (
      <div className="px-2 py-3 font-mono text-[11px] text-alert">
        {state.message}{' '}
        <button
          type="button"
          onClick={h.onRetry}
          className="lowercase text-accent hover:underline"
        >
          retry
        </button>
      </div>
    )
  if (state.groups.length === 0) return <Empty text="no checkpoints yet." />

  return (
    <>
      {state.groups.map((group) => (
        <GroupRow
          key={group.key}
          group={group}
          // Session-filtered view: undo targets a turn. A rare record
          // without one (external tooling) is listed but not actionable —
          // a global steps-undo could hit another session's newest record.
          canUndo={Boolean(group.turnId)}
          busy={h.undoingKey !== null}
          inFlight={h.undoingKey === group.key}
          onUndo={h.onUndo}
        />
      ))}
      {state.truncated ? (
        <p className="px-2 pt-1 font-mono text-[10px] lowercase text-ink-ghost">
          showing the most recent changes only.
        </p>
      ) : null}
    </>
  )
}

export function GroupRow({
  group,
  canUndo,
  busy,
  inFlight,
  onUndo,
}: {
  group: CheckpointGroup
  canUndo: boolean
  busy: boolean
  inFlight: boolean
  onUndo: (group: CheckpointGroup) => void
}) {
  return (
    <section className="border border-rule-2 bg-bg">
      <div className="flex items-start gap-2 px-2 py-1.5">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span
              className="font-mono text-[11px] text-ink"
              title={new Date(group.ts).toLocaleString()}
            >
              {formatRecordTime(group.ts)}
            </span>
            {group.isRevert ? (
              <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-accent">
                revert
              </span>
            ) : null}
          </div>
          <div className="mt-0.5 font-mono text-[10px] lowercase text-ink-ghost">
            {group.functionIds.join(', ')}
          </div>
          {group.files.length > 0 ? (
            <div className="mt-1 flex flex-wrap gap-x-2 gap-y-0.5">
              {group.files.map((f) => (
                <span
                  key={f}
                  title={f}
                  className="font-mono text-[10px] text-ink-faint"
                >
                  {basename(f)}
                </span>
              ))}
            </div>
          ) : null}
        </div>
        <button
          type="button"
          disabled={!canUndo || busy}
          title={
            canUndo
              ? undefined
              : 'this record has no turn attribution — undo it from the CLI'
          }
          onClick={() => onUndo(group)}
          className="shrink-0 lowercase text-ink-faint transition-colors hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
        >
          {inFlight ? '…' : group.isRevert ? 'redo' : 'undo'}
        </button>
      </div>
    </section>
  )
}

function Empty({ text }: { text: string }) {
  return (
    <div className="px-2 py-6 text-center font-mono text-[11px] lowercase text-ink-ghost">
      {text}
    </div>
  )
}
