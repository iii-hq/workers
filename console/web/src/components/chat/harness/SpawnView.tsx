import type { ReactNode } from 'react'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { Markdown } from '@/lib/markdown'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import {
  type SpawnRequest,
  safeParseRequest,
  spawnRequestSchema,
  spawnResponseSchema,
  taskText,
} from './parsers'

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
      <PaneHeader>task</PaneHeader>
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
        <PaneHeader>spawned child</PaneHeader>
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
        <PaneHeader>child result</PaneHeader>
        <GhostLine>· no result</GhostLine>
      </>
    )
  }

  if (typeof output === 'string') {
    return (
      <>
        <PaneHeader>child result</PaneHeader>
        <div className="px-3 py-2 bg-bg text-[13px] leading-[1.6]">
          <Markdown>{output}</Markdown>
        </div>
      </>
    )
  }

  return (
    <>
      <PaneHeader>child result · json</PaneHeader>
      <JsonHighlight
        code={JSON.stringify(output, null, 2)}
        className="max-h-80 overflow-auto"
        wrap
      />
    </>
  )
}
