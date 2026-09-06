import { Bot, Sparkles } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { FunctionTriggerCard } from '@/components/function-trigger/FunctionTriggerCard'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import type { TriggerRegistration } from '@/components/trigger-activity/model'
import { TriggerActivityCard } from '@/components/trigger-activity/TriggerActivityCard'
import { Caret } from '@/components/ui/Caret'
import { Chip } from '@/components/ui/Chip'
import { Prompt } from '@/components/ui/Prompt'
import { Card, CardBody, CardHeader } from '@/components/ui/Surface'
import { Markdown } from '@/lib/markdown'
import { invocationCommand, parseSlashInvocations } from '@/lib/slash-commands'
import { JsonHighlight } from '@/lib/syntax'
import type {
  AssistantMessage as AssistantMessageType,
  Message as MessageType,
  SubagentAppearance,
  SystemMessage as SystemMessageType,
  UserMessage as UserMessageType,
} from '@/types/chat'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import { AttachmentChip } from './AttachmentChip'
import { CopyMessageButton } from './CopyMessageButton'
import { MemoryChip } from './MemoryChip'
import { SystemNotice } from './SystemNotice'
import { ThoughtMessage } from './ThoughtMessage'
import './streaming-message.css'

interface MessageProps {
  message: MessageType
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
  /** Copy payload for an assistant turn (prose + its function calls). Lazy so
      the string is built on click, not on every streaming re-render. */
  copyText?: string | (() => string)
  /** Render function-call cards already expanded (showcase surfaces). */
  defaultOpenCalls?: boolean
  /** Registration detail for a trigger-fired or notification message
      (resolved in MessageList). */
  registration?: TriggerRegistration
  /** The machine-authored wake entry paired with a durable trigger record. */
  triggerNotification?: UserMessageType
  /** Current child-session identity for a direct spawn seed message. */
  spawnContext?: SpawnTaskContext
  /** Session agent profile name shown on assistant message headers. */
  agentName?: string
}

export interface SpawnTaskContext {
  title?: string | null
  model?: string | null
  appearance?: SubagentAppearance
}

export function Message({
  message,
  onResolveApproval,
  onAlwaysAllow,
  onResolveFilesystemAccess,
  onManageFilesystemAccess,
  workingDir,
  copyText,
  defaultOpenCalls,
  registration,
  triggerNotification,
  spawnContext,
  agentName,
}: MessageProps) {
  switch (message.role) {
    case 'user':
      return message.notification ? (
        <TriggerActivityCard
          notification={message}
          registration={registration}
          defaultOpen={defaultOpenCalls}
        />
      ) : message.reaction ? (
        <ReactionTaskMessage message={message} />
      ) : message.spawn ? (
        <SpawnTaskMessage message={message} context={spawnContext} />
      ) : message.validation ? (
        <ValidationNudgeMessage message={message} />
      ) : (
        <UserMessage message={message} />
      )
    case 'assistant':
      return (
        <AssistantMessage
          message={message}
          copyText={copyText}
          agentName={agentName}
        />
      )
    case 'thought':
      return <ThoughtMessage message={message} />
    case 'function-trigger': {
      const sessionId = message.sessionId
      const functionTriggerId = message.functionTriggerId
      let onApprove: (() => Promise<void>) | undefined
      let onDeny: (() => Promise<void>) | undefined
      let onAlwaysAllowHandler: (() => Promise<void>) | undefined
      let onResolveFilesystemAccessHandler:
        | ((action: FilesystemAccessAction) => Promise<void>)
        | undefined
      if (onResolveApproval && sessionId && functionTriggerId) {
        onApprove = () =>
          onResolveApproval(sessionId, functionTriggerId, 'allow')
        onDeny = () => onResolveApproval(sessionId, functionTriggerId, 'deny')
      }
      if (onAlwaysAllow && sessionId && functionTriggerId) {
        onAlwaysAllowHandler = () =>
          onAlwaysAllow(sessionId, functionTriggerId, message.functionId)
      }
      if (onResolveFilesystemAccess && sessionId && functionTriggerId) {
        onResolveFilesystemAccessHandler = (action) =>
          onResolveFilesystemAccess(sessionId, functionTriggerId, action)
      }
      return (
        <FunctionTriggerCard
          message={message}
          defaultOpen={defaultOpenCalls}
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
        <TriggerActivityCard
          record={message}
          registration={registration}
          notification={triggerNotification}
          defaultOpen={defaultOpenCalls}
        />
      ) : (
        <SystemNotice message={message} />
      )
  }
}

function CompactionMarker({ message }: { message: SystemMessageType }) {
  const tokens = message.tokensBefore ?? 0
  const summary = message.summaryText ?? ''
  return (
    <article className="my-2">
      <details className="group">
        <summary className="flex items-center gap-3 cursor-pointer list-none select-none">
          <span className="flex-1 h-px bg-edge" aria-hidden="true" />
          <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint flex items-center gap-2 group-hover:text-ink transition-colors">
            <span>Compacted</span>
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
          <span className="flex-1 h-px bg-edge" aria-hidden="true" />
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

export { parseNotification } from '@/components/trigger-activity/model'

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
 * HISTORICAL transcripts only: a trigger-fired task delivered into a session
 * back when bindings could target `harness::spawn`. New runs never produce
 * these — trigger delivery no longer creates agents — but old conversations
 * must keep rendering faithfully.
 */
function ReactionTaskMessage({ message }: { message: UserMessageType }) {
  const event = message.reactionEvent
  const hint = event ? reactionEventHint(event) : null
  return (
    <article className="flex flex-col items-start gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
        <Prompt symbol="⚡">Trigger · reaction task</Prompt>
      </header>
      <div className="max-w-[80%] border-l border-rule pl-4 pr-1 py-1 break-words text-ink-faint">
        <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
          {message.content}
        </Markdown>
        {event ? (
          <details className="mt-2 group">
            <summary className="cursor-pointer list-none select-none font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost group-hover:text-ink transition-colors">
              firing event
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
 * A validation nudge (`validation: true`): the harness re-prompting the turn
 * after the output contract or a `harness::hook::post-turn` validator
 * rejected its result. Labeled and left-aligned like the other
 * machine-authored user entries so it never reads as something the human
 * typed — the loop is visible AS a loop.
 */
function ValidationNudgeMessage({ message }: { message: UserMessageType }) {
  return (
    <article className="flex flex-col items-start gap-2">
      <header className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
        <Prompt symbol="⟳">Validator · corrective prompt</Prompt>
      </header>
      <div className="max-w-[80%] border-l border-rule pl-4 pr-1 py-1 break-words text-ink-faint">
        <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
          {message.content}
        </Markdown>
      </div>
    </article>
  )
}

/**
 * A direct `harness::spawn` seed task (`spawn: true`): the sub-agent's opening
 * input, but sent by the PARENT agent — labeled and left-aligned like a
 * reaction task so it never reads as something the human typed.
 */
function SpawnTaskMessage({
  message,
  context,
}: {
  message: UserMessageType
  context?: SpawnTaskContext
}) {
  const appearance = context?.appearance
  const title = appearance?.name.trim() || context?.title?.trim() || 'Sub-agent'
  const AgentIcon = appearance?.icon
    ? SUBAGENT_ICON_COMPONENTS[appearance.icon]
    : Bot

  return (
    <article
      className="w-full"
      data-message-role="spawn-task"
      aria-label={`${title} spawn task`}
    >
      <Card>
        <CardHeader className="border-b border-edge">
          <Sparkles aria-hidden className="size-4 shrink-0 stroke-accent" />
          <div className="flex min-w-0 items-center gap-2 font-mono text-[0.8125rem] font-medium tracking-[0.06em]">
            <span className="shrink-0">Spawn</span>
            {context?.model ? (
              <>
                <span aria-hidden className="shrink-0 text-ink-ghost">
                  ·
                </span>
                <span
                  className="min-w-0 truncate font-normal normal-case tracking-normal text-ink-faint"
                  title={context.model}
                >
                  {context.model}
                </span>
              </>
            ) : null}
          </div>
        </CardHeader>

        <div className="flex min-w-0 items-center gap-3 p-3 bg-ink-faint/3">
          <AgentIcon
            aria-hidden
            className="size-5 shrink-0 stroke-accent sm:size-4"
          />
          <div className="flex min-w-0 flex-1 flex-wrap align-center items-center gap-2 font-sans text-base sm:text-sm">
            <div
              className="min-w-0 truncate font-semibold text-ink"
              title={title}
            >
              {title}
            </div>
            <Chip tone="accent">Sub-agent</Chip>
          </div>
        </div>

        <div
          className="h-px bg-edge/40"
          style={{ width: 'calc(100% + 24px)', marginLeft: '-12px' }}
        ></div>

        <CardBody className="p-0">
          <div className="break-words p-4 text-ink-faint sm:p-3">
            <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
              {message.content}
            </Markdown>
          </div>
        </CardBody>
      </Card>
    </article>
  )
}

function UserMessage({ message }: { message: UserMessageType }) {
  const attachments = message.attachments ?? []
  /* A skill invocation already renders as a command pill inside the prose
     (the markdown turns `/skill:<id>` into one wherever it sits), so its
     expansion chip would only repeat the same name one row below the
     bubble. A chip whose command is not in the text (shouldn't happen)
     keeps the strip as a fallback. */
  const inline = new Set(
    parseSlashInvocations(message.content).map(invocationCommand),
  )
  const chips = attachments.filter(
    (a) => !(a.type === 'text/x-skill' && inline.has(a.name)),
  )
  return (
    <article
      className="group flex flex-col items-end gap-2"
      data-message-role="user"
    >
      <header className="flex items-center gap-2 font-sans text-base font-medium text-ink-faint sm:text-sm">
        {message.content ? (
          <CopyMessageButton
            text={message.content}
            className="opacity-100 sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
          />
        ) : null}
        <span>You</span>
      </header>
      <div className="max-w-[92%] break-words rounded-sm bg-surface px-3.5 py-2.5 sm:max-w-[80%]">
        <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
          {message.content}
        </Markdown>
      </div>
      {chips.length > 0 ? (
        <div className="flex max-w-[92%] flex-wrap justify-end gap-2 sm:max-w-[80%]">
          {chips.map((a) => (
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
  agentName,
}: {
  message: AssistantMessageType
  copyText?: string | (() => string)
  agentName?: string
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
      <header className="flex flex-wrap items-center gap-2 font-sans text-base text-ink-ghost sm:text-sm">
        {/* An agent worker names itself on the message; the harness's own
            turns do not. Fall back to the session's configured profile name,
            then to the literal "Agent" — a model id is not a name, since two
            agents can run the same model. */}
        <span className="font-medium text-ink-faint">
          {message.agent || agentName || 'Agent'}
        </span>
        {copySource !== undefined && !message.streaming ? (
          <CopyMessageButton
            text={copySource}
            className="opacity-100 sm:order-last sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
          />
        ) : null}
        {message.model ? (
          <span className="text-ink-ghost">· {message.model}</span>
        ) : null}
        {message.memory ? <MemoryChip memory={message.memory} /> : null}
      </header>
      <div className="pr-1">
        {message.content ? (
          <StreamingMarkdown
            content={message.content}
            streaming={!!message.streaming}
          />
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

const STREAMING_BLOCK_BURST_LIMIT = 3
const STREAMING_BLOCK_CHARACTER_LIMIT = 480

/**
 * Markdown can change the meaning of text that arrived earlier (for example,
 * when a closing fence completes a code block). Animating individual words
 * would therefore require splitting or replacing ReactMarkdown's DOM. Keep
 * existing nodes untouched and animate only genuinely appended top-level
 * blocks. Large snapshots stay immediate so reconnects and fast providers do
 * not build a visual queue behind the real transcript.
 */
export function shouldAnimateStreamingBlockBurst(
  blockCount: number,
  characterCount: number,
  reducedMotion: boolean,
): boolean {
  return (
    !reducedMotion &&
    blockCount > 0 &&
    blockCount <= STREAMING_BLOCK_BURST_LIMIT &&
    characterCount <= STREAMING_BLOCK_CHARACTER_LIMIT
  )
}

function StreamingMarkdown({
  content,
  streaming,
}: {
  content: string
  streaming: boolean
}) {
  const surfaceRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const surface = surfaceRef.current
    if (!streaming || !surface || typeof MutationObserver === 'undefined') {
      return
    }

    const markdownRoot = surface.firstElementChild
    if (!(markdownRoot instanceof HTMLElement)) return

    const observer = new MutationObserver((records) => {
      const appendedBlockSet = new Set<HTMLElement>()
      for (const record of records) {
        // Text appended to an existing paragraph remains instant. Only a new
        // semantic Markdown block receives motion, so old prose never fades or
        // becomes temporarily unselectable again.
        if (record.target !== markdownRoot || record.removedNodes.length > 0) {
          continue
        }
        for (const node of record.addedNodes) {
          if (node instanceof HTMLElement) appendedBlockSet.add(node)
        }
      }

      const appendedBlocks = [...appendedBlockSet]
      const topLevelBlocks = [...markdownRoot.children]
      const firstAppendIndex = topLevelBlocks.findIndex((block) =>
        appendedBlockSet.has(block as HTMLElement),
      )
      const isAppendedSuffix =
        firstAppendIndex >= 0 &&
        topLevelBlocks
          .slice(firstAppendIndex)
          .every((block) => appendedBlockSet.has(block as HTMLElement))

      const reducedMotion = window.matchMedia?.(
        '(prefers-reduced-motion: reduce)',
      ).matches
      const characterCount = appendedBlocks.reduce(
        (count, block) => count + (block.textContent?.length ?? 0),
        0,
      )
      if (
        !isAppendedSuffix ||
        !shouldAnimateStreamingBlockBurst(
          appendedBlocks.length,
          characterCount,
          !!reducedMotion,
        )
      ) {
        return
      }

      for (const block of appendedBlocks) {
        block.classList.add('assistant-stream-appended-block')
        const clearMotionClass = () => {
          block.classList.remove('assistant-stream-appended-block')
          block.removeEventListener('animationend', clearMotionClass)
          block.removeEventListener('animationcancel', clearMotionClass)
        }
        block.addEventListener('animationend', clearMotionClass, { once: true })
        block.addEventListener('animationcancel', clearMotionClass, {
          once: true,
        })
      }
    })

    observer.observe(markdownRoot, { childList: true })
    return () => observer.disconnect()
  }, [streaming])

  return (
    <div
      ref={surfaceRef}
      className={
        streaming && shouldAnimateStreamingBlockBurst(1, content.length, false)
          ? 'assistant-stream-surface is-entering'
          : undefined
      }
      data-assistant-stream-surface=""
    >
      <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
        {content}
      </Markdown>
    </div>
  )
}
