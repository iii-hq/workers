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
  /** Warns that undoing while a turn runs may fight the agent's writes. */
  sessionBusy?: boolean
}

type LoadState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; groups: CheckpointGroup[]; truncated: boolean }

const basename = (p: string): string => p.split('/').pop() || p

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
      setState({
        status: 'ready',
        groups: groupCheckpoints(res.records),
        truncated: res.truncated,
      })
    } catch (err) {
      setState({
        status: 'error',
        message: err instanceof Error ? err.message : String(err),
      })
    }
  }, [workingDir])

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
        const undone = await undoCheckpoint(
          workingDir,
          // Turn-less records can only be targeted by count, and only the
          // newest is offered (steps: 1) — see canUndo in render.
          group.turnId ? { turnId: group.turnId } : { steps: 1 },
        )
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
    [workingDir, load],
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
      {state.groups.map((group, index) => (
        <GroupRow
          key={group.key}
          group={group}
          // Turn-less records are only reversible when newest (steps: 1).
          canUndo={Boolean(group.turnId) || index === 0}
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
              {new Date(group.ts).toLocaleTimeString()}
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
              : 'only the most recent change can be undone without a turn id'
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
