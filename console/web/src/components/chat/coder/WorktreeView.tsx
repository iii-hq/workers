/**
 * `coder::worktree-add` / `coder::worktree-remove` — small lifecycle
 * cards for named git worktrees. Add reports where the worktree landed
 * (path + branch); remove reports the DISPOSITION: removed vs kept
 * because dirty (uncommitted work is never silently destroyed), and
 * whether the branch went with it.
 */
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  safeParseRequest,
  safeParseResponse,
  type WorktreeRemoveResponse,
  worktreeAddRequestSchema,
  worktreeAddResponseSchema,
  worktreeRemoveRequestSchema,
  worktreeRemoveResponseSchema,
} from './parsers'

interface WorktreeViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

export interface RemoveDisposition {
  label: string
  tone: 'accent' | 'warn' | 'alert' | 'default'
}

/** Human-readable outcome of a worktree remove. kept-dirty is the
    load-bearing state: the worktree survived BECAUSE it held
    uncommitted work — never let it read like a completed removal. */
export function removeDisposition(
  resp: WorktreeRemoveResponse,
): RemoveDisposition {
  if (!resp.removed) {
    return resp.dirty
      ? { label: 'kept — dirty', tone: 'warn' }
      : { label: 'not removed', tone: 'default' }
  }
  return resp.branch_deleted
    ? { label: 'removed · branch deleted', tone: 'accent' }
    : { label: 'removed · branch kept', tone: 'accent' }
}

function WorktreeCard({
  verb,
  name,
  running,
  runningLabel,
  children,
}: {
  verb: string
  name: string
  running?: boolean
  runningLabel: string
  children?: React.ReactNode
}) {
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[12.5px] text-ink">
          <span className="text-ink-faint">worktree </span>
          <span>{verb}</span>
        </span>
        <Chip label="name">{name}</Chip>
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · {runningLabel}
          </span>
        ) : null}
      </div>
      {children}
    </div>
  )
}

export function WorktreeAddView({
  input,
  output,
  running,
  preview,
}: WorktreeViewProps) {
  const req = safeParseRequest(worktreeAddRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(worktreeAddResponseSchema, output)
      : null

  return (
    <WorktreeCard
      verb="add"
      name={req.name}
      running={running}
      runningLabel="creating…"
    >
      {resp ? (
        <div className="px-3 py-2 flex flex-wrap items-center gap-1.5">
          <Chip label="path">{resp.path}</Chip>
          <Chip label="branch">{resp.branch}</Chip>
        </div>
      ) : null}
    </WorktreeCard>
  )
}

export function WorktreeRemoveView({
  input,
  output,
  running,
  preview,
}: WorktreeViewProps) {
  const req = safeParseRequest(worktreeRemoveRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(worktreeRemoveResponseSchema, output)
      : null

  const disposition = resp ? removeDisposition(resp) : null

  return (
    <WorktreeCard
      verb="remove"
      name={req.name}
      running={running}
      runningLabel="removing…"
    >
      {resp && disposition ? (
        <div className="px-3 py-2 flex flex-wrap items-center gap-1.5">
          <FooterPill tone={disposition.tone}>{disposition.label}</FooterPill>
          <Chip label="path">{resp.path}</Chip>
          <Chip label="branch">{resp.branch}</Chip>
          {resp.dirty && !resp.removed ? (
            <span className="font-mono text-[11px] text-warn">
              · uncommitted work left in place
            </span>
          ) : null}
        </div>
      ) : null}
    </WorktreeCard>
  )
}
