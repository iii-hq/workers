import { Caret } from '@/components/ui/Caret'
import { Prompt } from '@/components/ui/Prompt'
import { Markdown } from '@/lib/markdown'
import { cn } from '@/lib/utils'
import type {
  AssistantMessage as AssistantMessageType,
  Message as MessageType,
  UserMessage as UserMessageType,
} from '@/types/chat'
import { AttachmentChip } from './AttachmentChip'
import { FunctionCallMessage } from './FunctionCallMessage'
import { ThoughtMessage } from './ThoughtMessage'

interface MessageProps {
  message: MessageType
  onResolveApproval?: (
    sessionId: string,
    functionCallId: string,
    decision: 'allow' | 'deny',
  ) => Promise<void>
}

export function Message({ message, onResolveApproval }: MessageProps) {
  switch (message.role) {
    case 'user':
      return <UserMessage message={message} />
    case 'assistant':
      return <AssistantMessage message={message} />
    case 'thought':
      return <ThoughtMessage message={message} />
    case 'function-call': {
      const sessionId = message.sessionId
      const functionCallId = message.functionCallId
      let onApprove: (() => Promise<void>) | undefined
      let onDeny: (() => Promise<void>) | undefined
      if (onResolveApproval && sessionId && functionCallId) {
        onApprove = () => onResolveApproval(sessionId, functionCallId, 'allow')
        onDeny = () => onResolveApproval(sessionId, functionCallId, 'deny')
      }
      return (
        <FunctionCallMessage
          message={message}
          onApprove={onApprove}
          onDeny={onDeny}
        />
      )
    }
  }
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
