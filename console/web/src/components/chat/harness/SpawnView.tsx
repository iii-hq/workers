import {
  Activity,
  Bot,
  BrainCircuit,
  CircleAlert,
  LoaderCircle,
  Send,
} from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { ActivityMetadata } from '@/components/ui/ActivityMetadata'
import { ActivityStatus } from '@/components/ui/ActivityStatus'
import { OpenDetailsAffordance } from '@/components/ui/OpenDetailsAffordance'
import { useRelativeClock } from '@/hooks/use-relative-clock'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import { Markdown } from '@/lib/markdown'
import { formatElapsed } from '@/lib/relative-time'
import { fetchTranscript } from '@/lib/sessions/api'
import { subscribeSessionTranscript } from '@/lib/sessions/events'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import {
  type SpawnRequest,
  safeParseRequest,
  spawnRequestSchema,
  spawnResponseSchema,
  taskText,
} from './parsers'
import {
  activityFromAgentMessage,
  displayedSubagentActivity,
  latestSubagentActivity,
  resolveChildSessionId,
  type SubagentActivityKind,
  type SubagentActivitySignal,
} from './subagent-activity'

interface SpawnViewProps {
  input: unknown
  /** Already unwrapped once by the dispatcher — never `unwrapEnvelope` here
      (a child's json result may itself contain `content`/`details` keys). */
  output?: unknown
  running?: boolean
}

/**
 * `harness::spawn` — the sub-agent pending trigger. The request is the
 * child's task plus its policy (model, mode, turn budget, output contract,
 * function globs); the output is the child's final result: a markdown string
 * for a text contract, a structured value for a json contract, or the bare
 * `{ child_session_id, child_turn_id }` acknowledgement on a direct call.
 */
export function SpawnView({ input, output, running }: SpawnViewProps) {
  const req = safeParseRequest(spawnRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="child running…" variant="default" />
          <SpawnChips req={req} />
        </MetaRow>
        <TaskPane task={req.task} />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · waiting for the child to finish…
        </div>
      </div>
    )
  }

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="spawned" variant="accent" />
        <SpawnChips req={req} />
      </MetaRow>
      <TaskPane task={req.task} />
      <ResultPane output={output} />
    </div>
  )
}

interface SpawnActivityDisplayProps {
  input: unknown
  output?: unknown
  parentSessionId?: string
  functionTriggerId?: string
  createdAt?: number
}

/** Compact live surface for a spawned child. The session directory supplies
 * coarse lifecycle updates for every child; a scoped transcript subscription
 * refines a working child into thinking, tool work, or message streaming. */
export function SpawnActivityDisplay({
  input,
  output,
  parentSessionId,
  functionTriggerId,
  createdAt,
}: SpawnActivityDisplayProps) {
  const req = safeParseRequest(spawnRequestSchema, input)
  const response = spawnResponseSchema.safeParse(output)
  const ctx = useConversationsCtxOptional()
  const childSessionId = resolveChildSessionId({
    responseSessionId: response.success
      ? response.data.child_session_id
      : undefined,
    requestSessionId: req?.session_id,
    parentSessionId,
    functionTriggerId,
    conversations: ctx?.conversations ?? [],
  })
  const child = ctx?.conversations.find(
    (conversation) => conversation.id === childSessionId,
  )
  const signal = useLiveSubagentActivity(childSessionId, child?.status)
  const activity = displayedSubagentActivity(child?.status, signal)
  const clock = useRelativeClock(
    child?.createdAt ?? createdAt ?? signal?.timestamp ?? child?.updatedAt,
  )

  if (!req) return null
  const task = taskText(req.task) ?? 'Waiting for the assigned task.'
  const childTitle =
    child?.title && child.title !== child.id ? child.title : task
  const open =
    ctx && child
      ? () => {
          ctx.select(child.id)
        }
      : undefined

  return (
    <SpawnActivityCard
      title={childTitle}
      task={task}
      status={activity}
      sessionId={childSessionId}
      createdAt={child?.createdAt ?? createdAt}
      activityAt={signal?.timestamp ?? child?.updatedAt ?? child?.createdAt}
      now={clock}
      onOpen={open}
    />
  )
}

function useLiveSubagentActivity(
  sessionId: string | null,
  status: 'idle' | 'working' | 'done' | 'error' | undefined,
): SubagentActivitySignal | null {
  const [signal, setSignal] = useState<SubagentActivitySignal | null>(null)

  useEffect(() => {
    if (status !== 'working') setSignal(null)
  }, [status])

  useEffect(() => {
    setSignal(null)
    if (!sessionId) return
    let cancelled = false
    let off: (() => void) | null = null
    const accept = (next: SubagentActivitySignal | null) => {
      if (!next || cancelled) return
      setSignal((current) =>
        !current || next.timestamp >= current.timestamp ? next : current,
      )
    }

    void fetchTranscript(sessionId)
      .then((items) => accept(latestSubagentActivity(items)))
      .catch(() => {})
    void getIiiClient()
      .then((client) => {
        if (cancelled) return
        off = subscribeSessionTranscript(client, sessionId, {
          onMessageAdded: (event) =>
            accept(activityFromAgentMessage(event.message, event.timestamp)),
          onMessageUpdated: (event) =>
            accept(activityFromAgentMessage(event.message, event.timestamp)),
        })
      })
      .catch(() => {})

    return () => {
      cancelled = true
      off?.()
    }
  }, [sessionId])

  return signal
}

interface SpawnActivityCardProps {
  title: string
  task: string
  status: SubagentActivityKind
  sessionId?: string | null
  createdAt?: number
  activityAt?: number
  /** Deterministic clock for stories/tests; live surfaces provide a timer. */
  now?: number
  onOpen?: () => void
}

/** Props-only presentation exported for fixtures and deterministic state tests. */
export function SpawnActivityCard({
  title,
  task,
  status,
  sessionId,
  createdAt,
  activityAt,
  now = Date.now(),
  onOpen,
}: SpawnActivityCardProps) {
  const activityAge = formatElapsed(activityAt, now)
  const content = (
    <div className="grid min-w-0 gap-4 @xl:grid-cols-[minmax(0,1fr)_auto] @xl:items-center">
      <div className="flex min-w-0 items-start gap-3">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-md bg-accent-muted text-accent sm:size-9">
          <Bot aria-hidden className="size-5" strokeWidth={2.25} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <div className="font-sans text-base font-semibold text-ink sm:text-sm">
              New sub-agent
            </div>
            <span className="rounded-md bg-accent-muted px-2 py-0.5 font-sans text-sm font-medium text-accent sm:text-xs">
              New
            </span>
          </div>
          <div className="mt-1 line-clamp-2 text-pretty font-sans text-base leading-6 text-ink-faint sm:text-sm sm:leading-5">
            {task}
          </div>
          <ActivityMetadata
            className="mt-3"
            createdAt={createdAt}
            identifier={sessionId}
            now={now}
          />
        </div>
      </div>

      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 border-t border-rule-2 pt-3 @xl:flex @xl:flex-col @xl:items-stretch @xl:border-t-0 @xl:pt-0">
        <SubagentStatus status={status} activityAge={activityAge} />
        {onOpen ? (
          <OpenDetailsAffordance className="group-hover/subagent:bg-surface-hover" />
        ) : null}
      </div>
    </div>
  )
  const className = cn(
    '@container w-full rounded-md bg-panel-raised px-4 py-4 text-left shadow-raised sm:px-3 sm:py-3',
    onOpen &&
      'group/subagent cursor-pointer transition-colors hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
  )

  if (onOpen) {
    return (
      <button
        type="button"
        className={className}
        data-subagent-status={status}
        aria-label={`Open sub-agent session ${sessionId ?? title}`}
        title={sessionId ? `Open sub-agent session ${sessionId}` : undefined}
        onClick={onOpen}
      >
        {content}
      </button>
    )
  }
  return (
    <div className={className} data-subagent-status={status}>
      {content}
    </div>
  )
}

function SubagentStatus({
  status,
  activityAge,
}: {
  status: SubagentActivityKind
  activityAge: string | null
}) {
  const label =
    status === 'thinking'
      ? 'Thinking'
      : status === 'messaging'
        ? 'Sending a message'
        : status === 'working'
          ? 'Working'
          : status === 'error'
            ? 'Needs attention'
            : 'Active'
  const detail =
    activityAge == null
      ? label
      : activityAge === 'just now'
        ? `${label} now`
        : `${label} for ${activityAge}`
  const Icon =
    status === 'thinking'
      ? BrainCircuit
      : status === 'messaging'
        ? Send
        : status === 'working'
          ? LoaderCircle
          : status === 'error'
            ? CircleAlert
            : Activity

  return (
    <ActivityStatus
      label={label}
      detail={activityAge ? detail : null}
      icon={Icon}
      tone={
        status === 'active'
          ? 'positive'
          : status === 'error'
            ? 'danger'
            : status === 'thinking' || status === 'messaging'
              ? 'accent'
              : 'neutral'
      }
      motion={
        status === 'working'
          ? 'spin'
          : status === 'thinking' || status === 'messaging'
            ? 'pulse'
            : 'none'
      }
    />
  )
}

/** Compact preview rendered while a `harness::spawn` call sits in the
    approval gate: the policy chips are the point. Tolerates a clipped
    `arguments_excerpt` (every field optional). */
export function SpawnPreview({ input }: { input: unknown }) {
  const req = safeParseRequest(spawnRequestSchema, input)
  if (!req) return null
  return (
    <div className="border-b border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="permission to spawn" variant="warn" />
        <SpawnChips req={req} />
      </MetaRow>
      <TaskPane task={req.task} />
    </div>
  )
}

/* ---------------- pieces ---------------- */

function KvChip({ k, v, warn }: { k: string; v: ReactNode; warn?: boolean }) {
  return (
    <Chip className={cn(warn && 'border-warn/40')}>
      <span
        className={cn(
          'uppercase tracking-[0.06em]',
          warn ? 'text-warn' : 'text-ink-faint',
        )}
      >
        {k}
      </span>
      <span className="ml-1 text-ink break-all">{v}</span>
    </Chip>
  )
}

function SpawnChips({ req }: { req: SpawnRequest }) {
  const opts = req.options
  return (
    <>
      {req.model ? <KvChip k="model" v={req.model} /> : null}
      {req.provider ? <KvChip k="provider" v={req.provider} /> : null}
      {opts?.mode ? <KvChip k="mode" v={opts.mode} /> : null}
      {typeof opts?.max_turns === 'number' ? (
        <KvChip
          k="turns"
          v={<span className="tabular-nums">{opts.max_turns}</span>}
        />
      ) : null}
      {opts?.thinking_level ? (
        <KvChip k="thinking" v={opts.thinking_level} />
      ) : null}
      {opts?.output ? <KvChip k="output" v={opts.output.type} /> : null}
      {opts?.functions?.expose ? (
        <KvChip k="expose" v={opts.functions.expose} />
      ) : null}
      {opts?.functions?.allow?.length ? (
        <KvChip k="allow" v={opts.functions.allow.join(' ')} />
      ) : null}
      {opts?.functions?.deny?.length ? (
        <KvChip k="deny" v={opts.functions.deny.join(' ')} warn />
      ) : null}
      {typeof opts?.max_children === 'number' ? (
        <KvChip
          k="children"
          v={<span className="tabular-nums">{opts.max_children}</span>}
        />
      ) : null}
      {req.session_id ? (
        <KvChip k="session" v={<SessionLink sessionId={req.session_id} />} />
      ) : null}
    </>
  )
}

/** Session id that jumps to the child conversation when the console knows it
    (same `select` the sidebar tree uses). Plain text outside the provider
    (Storybook) or when the session isn't in the conversation list. */
function SessionLink({ sessionId }: { sessionId: string }) {
  const ctx = useConversationsCtxOptional()
  const known = ctx?.conversations.some((c) => c.id === sessionId)
  if (!ctx || !known) return <>{sessionId}</>
  return (
    <button
      type="button"
      onClick={() => ctx.select(sessionId)}
      className="text-accent hover:underline cursor-pointer break-all text-left"
      title="open child session"
    >
      {sessionId}
    </button>
  )
}

function PaneHeader({ children }: { children: ReactNode }) {
  return (
    <div className="bg-paper-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
      {children}
    </div>
  )
}

function GhostLine({ children }: { children: ReactNode }) {
  return (
    <div className="px-3 py-2 font-mono text-[11.5px] text-ink-ghost">
      {children}
    </div>
  )
}

function TaskPane({ task }: { task: SpawnRequest['task'] }) {
  const text = taskText(task)
  return (
    <>
      <PaneHeader>Task</PaneHeader>
      {text ? (
        <div className="px-3 py-2 border-b border-rule-2 bg-bg text-[13px] leading-[1.6]">
          <Markdown>{text}</Markdown>
        </div>
      ) : (
        <GhostLine>· no task</GhostLine>
      )}
    </>
  )
}

function ResultPane({ output }: { output: unknown }) {
  const direct = spawnResponseSchema.safeParse(output)
  if (direct.success) {
    return (
      <>
        <PaneHeader>Spawned child</PaneHeader>
        <ActionLine symbol="→" tone="ink">
          <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mr-2">
            session
          </span>
          <span className="font-mono text-[12px]">
            <SessionLink sessionId={direct.data.child_session_id} />
          </span>
        </ActionLine>
        <ActionLine symbol="→" tone="ink">
          <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mr-2">
            turn
          </span>
          <span className="font-mono text-[12px]">
            {direct.data.child_turn_id}
          </span>
        </ActionLine>
      </>
    )
  }

  if (output == null || output === '') {
    return (
      <>
        <PaneHeader>Child result</PaneHeader>
        <GhostLine>· no result</GhostLine>
      </>
    )
  }

  if (typeof output === 'string') {
    return (
      <>
        <PaneHeader>Child result</PaneHeader>
        <div className="px-3 py-2 bg-bg text-[13px] leading-[1.6]">
          <Markdown>{output}</Markdown>
        </div>
      </>
    )
  }

  return (
    <>
      <PaneHeader>Child result · JSON</PaneHeader>
      <JsonHighlight
        code={JSON.stringify(output, null, 2)}
        className="max-h-80 overflow-auto"
        wrap
      />
    </>
  )
}
