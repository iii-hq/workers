import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'
import {
  FunctionTriggerCard,
  isErrorOutput,
} from '@/components/function-trigger/FunctionTriggerCard'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { FunctionTriggerMessage as FunctionTriggerMessageType } from '@/types/chat'

interface FunctionTriggerGroupProps {
  messages: FunctionTriggerMessageType[]
  /** Force the group open. Used by the examples showcase. */
  defaultOpen?: boolean
  /**
   * Forwarded from `MessageList` so a pending approval landing on any
   * child call still wires through to `approval::resolve`. Each child's
   * approve/deny is bound to that child's `sessionId` + `functionTriggerId`.
   */
  onResolveApproval?: (
    sessionId: string,
    functionTriggerId: string,
    decision: 'allow' | 'deny',
  ) => Promise<void>
  onAlwaysAllow?: (
    sessionId: string,
    functionTriggerId: string,
    functionId: string,
  ) => Promise<void>
  onResolveFilesystemAccess?: (
    sessionId: string,
    functionTriggerId: string,
    action: FilesystemAccessAction,
  ) => Promise<void>
  onManageFilesystemAccess?: () => void
  workingDir?: string | null
}

type Tone = 'warn' | 'accent' | 'alert' | 'ink'

interface GroupStatus {
  tone: Tone
  pulse: boolean
  label: ReactNode
}

/**
 * Derive the single header line + dot tone that summarises the group.
 * Priority: pending approval > running > errored > done. Matches the
 * `FunctionTriggerMessage` dot conventions so the group reads as "one of
 * these" rather than a new visual language.
 */
function deriveStatus(messages: FunctionTriggerMessageType[]): GroupStatus {
  const total = messages.length

  const pending = messages.find((m) => m.pendingApproval)
  if (pending) {
    return {
      tone: 'warn',
      pulse: false,
      label: (
        <>
          permission to trigger{' '}
          <span className="text-accent italic font-semibold">ƒ</span>{' '}
          <span className="text-ink">{pending.functionId}</span>
        </>
      ),
    }
  }

  const runningIdx = messages.findIndex((m) => m.running)
  if (runningIdx >= 0) {
    const running = messages[runningIdx]
    return {
      tone: 'accent',
      pulse: true,
      label: (
        <>
          function <span className="tabular-nums">{runningIdx + 1}</span> of{' '}
          <span className="tabular-nums">{total}</span>:{' '}
          <span className="text-accent italic font-semibold">ƒ</span>{' '}
          {running.unresolvedTarget ? (
            <span className="text-ink-faint">…</span>
          ) : (
            <span className="text-ink">{running.functionId}</span>
          )}
        </>
      ),
    }
  }

  const failed = messages.filter((m) => isErrorOutput(m.output)).length
  if (failed > 0) {
    return {
      tone: 'alert',
      pulse: false,
      label: (
        <>
          <span className="tabular-nums">{failed}</span>{' '}
          {failed === 1 ? 'function' : 'functions'} failed
          {failed < total ? (
            <span className="text-ink-faint">
              {' '}
              of <span className="tabular-nums">{total}</span>
            </span>
          ) : null}
        </>
      ),
    }
  }

  const sum = messages.reduce((acc, m) => acc + (m.durationMs ?? 0), 0)
  return {
    tone: 'ink',
    pulse: false,
    label: (
      <>
        <span className="tabular-nums">{total}</span> functions for{' '}
        <span className="tabular-nums">{sum}</span>ms
      </>
    ),
  }
}

/**
 * Open if anything in the group needs attention — that's the only state
 * where the user can't infer what's happening from the one-line header.
 */
function hasConcerningChild(messages: FunctionTriggerMessageType[]): boolean {
  return messages.some(
    (m) => m.pendingApproval || m.running || isErrorOutput(m.output),
  )
}

export function FunctionTriggerGroup({
  messages,
  defaultOpen,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
}: FunctionTriggerGroupProps) {
  const status = deriveStatus(messages)
  const concerning = hasConcerningChild(messages)
  const [open, setOpen] = useState(defaultOpen ?? concerning)

  /* Re-open when something needs attention even mid-life: a new
     pending-approval, a transition into running, or an error payload
     landing on a previously-collapsed group. Going `concerning → calm`
     intentionally does NOT auto-close — once the user is reading the
     details, we keep them visible. */
  useEffect(() => {
    if (concerning) setOpen(true)
  }, [concerning])

  /* The group carries the same accent-tinted raised chrome as a standalone
     call card, so a run of calls reads as one special container. Embedded
     children inherit it; their internal label strips (`bg-paper-2`) and code
     panes (`bg-bg`) sit one and two layers above, creating a clear depth
     hierarchy in both light and dark themes. */
  return (
    <div className="function-trigger-surface fcall-chrome rounded-md overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={cn(
          'w-full flex items-center justify-between gap-3 px-3 py-2 cursor-pointer text-left',
          'hover:bg-surface-hover/50 transition-colors',
        )}
      >
        <span className="flex items-center gap-2 min-w-0">
          <StatusDot
            tone={status.tone}
            pulse={status.pulse}
            className="shrink-0"
          />
          <span className="font-mono text-[13px] text-ink truncate">
            {status.label}
          </span>
        </span>
        <span
          aria-hidden
          className={cn(
            'text-ink-ghost shrink-0 transition-transform duration-150 inline-block',
            open && 'rotate-90',
          )}
        >
          ▸
        </span>
      </button>

      {open ? (
        <div className="border-t border-rule-2 divide-y divide-rule-2">
          {messages.map((m) => {
            const sessionId = m.sessionId
            const functionTriggerId = m.functionTriggerId
            let onApprove: (() => Promise<void>) | undefined
            let onDeny: (() => Promise<void>) | undefined
            let onAlwaysAllowHandler: (() => Promise<void>) | undefined
            let onResolveFilesystemAccessHandler:
              | ((action: FilesystemAccessAction) => Promise<void>)
              | undefined
            if (onResolveApproval && sessionId && functionTriggerId) {
              onApprove = () =>
                onResolveApproval(sessionId, functionTriggerId, 'allow')
              onDeny = () =>
                onResolveApproval(sessionId, functionTriggerId, 'deny')
            }
            if (onAlwaysAllow && sessionId && functionTriggerId) {
              onAlwaysAllowHandler = () =>
                onAlwaysAllow(sessionId, functionTriggerId, m.functionId)
            }
            if (onResolveFilesystemAccess && sessionId && functionTriggerId) {
              onResolveFilesystemAccessHandler = (action) =>
                onResolveFilesystemAccess(sessionId, functionTriggerId, action)
            }
            return (
              <FunctionTriggerCard
                key={m.id}
                message={m}
                onApprove={onApprove}
                onDeny={onDeny}
                onAlwaysAllow={onAlwaysAllowHandler}
                onResolveFilesystemAccess={onResolveFilesystemAccessHandler}
                onManageFilesystemAccess={onManageFilesystemAccess}
                workingDir={workingDir}
                embedded
              />
            )
          })}
        </div>
      ) : null}
    </div>
  )
}
