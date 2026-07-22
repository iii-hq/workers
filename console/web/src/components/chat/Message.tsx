import { FunctionCallCard } from '@/components/function-call/FunctionCallCard'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { Caret } from '@/components/ui/Caret'
import { Prompt } from '@/components/ui/Prompt'
import { Markdown } from '@/lib/markdown'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type {
  AssistantMessage as AssistantMessageType,
  Message as MessageType,
  SystemMessage as SystemMessageType,
  UserMessage as UserMessageType,
} from '@/types/chat'
import { AttachmentChip } from './AttachmentChip'
import { CopyMessageButton } from './CopyMessageButton'
import { MemoryChip } from './MemoryChip'
import { ThoughtMessage } from './ThoughtMessage'

interface MessageProps {
  message: MessageType
  onResolveApproval?: (
    sessionId: string,
    functionCallId: string,
    decision: 'allow' | 'deny',
  ) => Promise<void>
  onAlwaysAllow?: (
    sessionId: string,
    functionCallId: string,
    functionId: string,
  ) => Promise<void>
  onResolveFilesystemAccess?: (
    sessionId: string,
    functionCallId: string,
    action: FilesystemAccessAction,
  ) => Promise<void>
  onManageFilesystemAccess?: () => void
  workingDir?: string | null
  /** Copy payload for an assistant turn (prose + its function calls). Lazy so
      the string is built on click, not on every streaming re-render. */
  copyText?: string | (() => string)
}

export function Message({
  message,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
  copyText,
}: MessageProps) {
  switch (message.role) {
    case 'user':
      return message.notification ? (
        <NotificationMessage message={message} />
      ) : message.reaction ? (
        <ReactionTaskMessage message={message} />
      ) : message.spawn ? (
        <SpawnTaskMessage message={message} />
      ) : (
        <UserMessage message={message} />
      )
    case 'assistant':
      return <AssistantMessage message={message} copyText={copyText} />
    case 'thought':
      return <ThoughtMessage message={message} />
    case 'function-call': {
      const sessionId = message.sessionId
      const functionCallId = message.functionCallId
      let onApprove: (() => Promise<void>) | undefined
      let onDeny: (() => Promise<void>) | undefined
      let onAlwaysAllowHandler: (() => Promise<void>) | undefined
      let onResolveFilesystemAccessHandler:
        | ((action: FilesystemAccessAction) => Promise<void>)
        | undefined
      if (onResolveApproval && sessionId && functionCallId) {
        onApprove = () => onResolveApproval(sessionId, functionCallId, 'allow')
        onDeny = () => onResolveApproval(sessionId, functionCallId, 'deny')
      }
      if (onAlwaysAllow && sessionId && functionCallId) {
        onAlwaysAllowHandler = () =>
          onAlwaysAllow(sessionId, functionCallId, message.functionId)
      }
      if (onResolveFilesystemAccess && sessionId && functionCallId) {
        onResolveFilesystemAccessHandler = (action) =>
          onResolveFilesystemAccess(sessionId, functionCallId, action)
      }
      return (
        <FunctionCallCard
          message={message}
          onApprove={onApprove}
          onDeny={onDeny}
          onAlwaysAllow={onAlwaysAllowHandler}
          onResolveFilesystemAccess={onResolveFilesystemAccessHandler}
          onManageFilesystemAccess={onManageFilesystemAccess}
          workingDir={workingDir}
        />
      )
    }
    case 'system':
      return message.kind === 'compaction' ? (
        <CompactionMarker message={message} />
      ) : message.kind === 'trigger-fired' ? (
        <TriggerFiredNotice message={message} />
      ) : (
        <SystemNotice message={message} />
      )
  }
}

function SystemNotice({ message }: { message: SystemMessageType }) {
  const tone = message.tone ?? 'info'
  const toneCls =
    tone === 'error'
      ? 'border-l-danger text-danger'
      : tone === 'warn'
        ? 'border-l-warn text-warn'
        : 'border-l-rule text-ink-faint'
  return (
    <article
      className={cn(
        'border-l-2 pl-3 py-1 font-mono text-[12px] uppercase tracking-[0.04em]',
        toneCls,
      )}
    >
      {message.content}
    </article>
  )
}

function CompactionMarker({ message }: { message: SystemMessageType }) {
  const tokens = message.tokensBefore ?? 0
  const summary = message.summaryText ?? ''
  return (
    <article className="my-2">
      <details className="group">
        <summary className="flex items-center gap-3 cursor-pointer list-none select-none">
          <span className="flex-1 h-px bg-rule" aria-hidden="true" />
          <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint flex items-center gap-2 group-hover:text-ink transition-colors">
            <span>compacted</span>
            {tokens > 0 ? (
              <>
                <span className="text-ink-ghost">·</span>
                <span className="tabular-nums">
                  {tokens.toLocaleString()} tokens
                </span>
              </>
            ) : null}
            <span className="text-ink-ghost normal-case tracking-normal text-[10px]">
              show summary
            </span>
          </span>
          <span className="flex-1 h-px bg-rule" aria-hidden="true" />
        </summary>
        {summary ? (
          <pre className="mt-3 mx-9 p-3 bg-panel border border-rule-2 font-mono text-[11px] leading-relaxed text-ink-faint whitespace-pre-wrap">
            {summary}
          </pre>
        ) : null}
      </details>
    </article>
  )
}

function NotificationMessage({ message }: { message: UserMessageType }) {
  return (
    <article className="border-l-2 border-l-rule pl-3 py-1 font-mono text-[12px] text-ink-faint flex items-start gap-2">
      <span aria-hidden="true">🔔</span>
      <span className="break-words">{message.content}</span>
    </article>
  )
}

/**
 * A subscription fire (`kind: 'trigger-fired'`): a turn-less notice that a
 * registered trigger fired — a state/cron spawn, a notify wake, or a join
 * edge. `message.content` is the pre-rendered one-liner (name · action).
 */
function TriggerFiredNotice({ message }: { message: SystemMessageType }) {
  return (
    <article className="border-l-2 border-l-rule pl-3 py-1 font-mono text-[12px] text-ink-faint flex items-start gap-2">
      <span aria-hidden="true">⚡</span>
      <span className="break-words">
        <span className="uppercase tracking-[0.04em] text-ink-ghost">
          trigger fired
        </span>
        {' · '}
        {message.content}
      </span>
    </article>
  )
}

/**
 * The one-line hint for a reaction's collapsed payload: the firing session
 * and status for an event, the predecessor keys for a join's inputs.
 */
function reactionEventHint(event: {
  label: 'event' | 'inputs'
  json: string
}): string | null {
  try {
    const v = JSON.parse(event.json) as Record<string, unknown>
    if (v === null || typeof v !== 'object') return null
    if (event.label === 'inputs') {
      const keys = Object.keys(v)
      return keys.length > 0 ? keys.join(' + ') : null
    }
    const parts = [v.session_id, v.status].filter(
      (x): x is string => typeof x === 'string',
    )
    return parts.length > 0 ? parts.join(' · ') : null
  } catch {
    return null
  }
}

/**
 * A react-fired task delivered into this session (`harness::react`): the
 * turn's input, but machine-sent — labeled "trigger" and left-aligned so it
 * never reads as something the human typed. The appended firing event (or
 * join inputs) collapses to a summary line, expandable to highlighted JSON.
 */
function ReactionTaskMessage({ message }: { message: UserMessageType }) {
  const event = message.reactionEvent
  const hint = event ? reactionEventHint(event) : null
  return (
    <article className="flex flex-col items-start gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
        <Prompt symbol="⚡">trigger · reaction task</Prompt>
      </header>
      <div className="max-w-[80%] border-l border-rule pl-4 pr-1 py-1 break-words text-ink-faint">
        <Markdown>{message.content}</Markdown>
        {event ? (
          <details className="mt-2 group">
            <summary className="cursor-pointer list-none select-none font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost group-hover:text-ink transition-colors">
              {event.label === 'inputs' ? 'join inputs' : 'firing event'}
              {hint ? ` · ${hint}` : ''}
              <span className="normal-case tracking-normal text-[10px]">
                {' '}
                · show json
              </span>
            </summary>
            <div className="mt-1 max-h-64 overflow-auto border border-rule-2">
              <JsonHighlight code={event.json} wrap />
            </div>
          </details>
        ) : null}
      </div>
    </article>
  )
}

/**
 * A direct `harness::spawn` seed task (`spawn: true`): the sub-agent's opening
 * input, but sent by the PARENT agent — labeled and left-aligned like a
 * reaction task so it never reads as something the human typed.
 */
function SpawnTaskMessage({ message }: { message: UserMessageType }) {
  return (
    <article className="flex flex-col items-start gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
        <Prompt symbol="⚙">spawn · sub-agent task</Prompt>
      </header>
      <div className="max-w-[80%] border-l border-rule pl-4 pr-1 py-1 break-words text-ink-faint">
        <Markdown>{message.content}</Markdown>
      </div>
    </article>
  )
}

function UserMessage({ message }: { message: UserMessageType }) {
  return (
    <article
      className="group flex flex-col items-end gap-2"
      data-message-role="user"
    >
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost flex items-center gap-2">
        {message.content ? (
          <CopyMessageButton
            text={message.content}
            className="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-[opacity,color]"
          />
        ) : null}
        <Prompt symbol="$">you</Prompt>
      </header>
      <div
        className={cn(
          'max-w-[80%] border-l border-rule pl-4 pr-1 py-1',
          'break-words',
        )}
      >
        <Markdown>{message.content}</Markdown>
      </div>
      {message.attachments && message.attachments.length > 0 ? (
        <div className="flex flex-wrap gap-2 justify-end max-w-[80%]">
          {message.attachments.map((a) => (
            <AttachmentChip key={a.id} attachment={a} />
          ))}
        </div>
      ) : null}
    </article>
  )
}

function AssistantMessage({
  message,
  copyText,
}: {
  message: AssistantMessageType
  copyText?: string | (() => string)
}) {
  const showCaret = !!message.streaming
  // A tool-only turn has no prose but still carries a copy payload (its
  // function calls) via copyText; direct renders without a list-provided
  // payload fall back to the prose gate.
  const copySource = copyText ?? (message.content || undefined)
  return (
    <article
      className="group flex flex-col gap-2"
      data-message-role="assistant"
    >
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost flex items-center gap-2 flex-wrap">
        <Prompt symbol=">">assistant</Prompt>
        {message.model ? (
          <span className="text-ink-ghost">· {message.model}</span>
        ) : null}
        {message.mode ? (
          <span className="text-ink-ghost">· {message.mode}</span>
        ) : null}
        {message.memory ? <MemoryChip memory={message.memory} /> : null}
        {copySource !== undefined && !message.streaming ? (
          <CopyMessageButton
            text={copySource}
            className="opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-[opacity,color]"
          />
        ) : null}
      </header>
      <div className="pr-1">
        {message.content ? (
          <Markdown>{message.content}</Markdown>
        ) : (
          <div className="font-mono text-[13px] italic thinking-shimmer">
            thinking…
          </div>
        )}
        {showCaret ? <Caret className="ml-0.5" /> : null}
      </div>
    </article>
  )
}
