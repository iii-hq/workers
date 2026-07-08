import { FunctionCallCard } from '@/components/function-call/FunctionCallCard'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import { Caret } from '@/components/ui/Caret'
import { Prompt } from '@/components/ui/Prompt'
import { Markdown } from '@/lib/markdown'
import { cn } from '@/lib/utils'
import type {
  AssistantMessage as AssistantMessageType,
  Message as MessageType,
  SystemMessage as SystemMessageType,
  UserMessage as UserMessageType,
} from '@/types/chat'
import { AttachmentChip } from './AttachmentChip'
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
}

export function Message({
  message,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
}: MessageProps) {
  switch (message.role) {
    case 'user':
      return message.notification ? (
        <NotificationMessage message={message} />
      ) : (
        <UserMessage message={message} />
      )
    case 'assistant':
      return <AssistantMessage message={message} />
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

function UserMessage({ message }: { message: UserMessageType }) {
  return (
    <article className="flex flex-col items-end gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
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

function AssistantMessage({ message }: { message: AssistantMessageType }) {
  const showCaret = !!message.streaming
  return (
    <article className="flex flex-col gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost flex items-center gap-2">
        <Prompt symbol=">">assistant</Prompt>
        {message.model ? (
          <span className="text-ink-ghost">· {message.model}</span>
        ) : null}
        {message.mode ? (
          <span className="text-ink-ghost">· {message.mode}</span>
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
