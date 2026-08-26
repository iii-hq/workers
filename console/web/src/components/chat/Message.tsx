import { Blocks, Bot, Sparkles } from 'lucide-react'
import { FunctionTriggerCard } from '@/components/function-trigger/FunctionTriggerCard'
import type { FilesystemAccessAction } from '@/components/permissions/FilesystemAccessPrompt'
import type { TriggerRegistration } from '@/components/trigger-activity/model'
import { TriggerActivityCard } from '@/components/trigger-activity/TriggerActivityCard'
import { Caret } from '@/components/ui/Caret'
import { Chip } from '@/components/ui/Chip'
import { Prompt } from '@/components/ui/Prompt'
import { Card, CardBody, CardHeader } from '@/components/ui/Surface'
import { Markdown } from '@/lib/markdown'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type {
  AssistantMessage as AssistantMessageType,
  Attachment,
  Message as MessageType,
  SubagentAppearance,
  SystemMessage as SystemMessageType,
  UserMessage as UserMessageType,
} from '@/types/chat'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import { AttachmentChip, formatSize } from './AttachmentChip'
import { CopyMessageButton } from './CopyMessageButton'
import { MemoryChip } from './MemoryChip'
import { ThoughtMessage } from './ThoughtMessage'

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
      return <AssistantMessage message={message} copyText={copyText} />
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

function SystemNotice({ message }: { message: SystemMessageType }) {
  const tone = message.tone ?? 'info'
  const detailRows: Array<[string, string]> = message.technicalDetails
    ? Object.entries(message.technicalDetails).flatMap(([key, value]) =>
        typeof value === 'string' && value.length > 0 ? [[key, value]] : [],
      )
    : []
  const structured = Boolean(message.nextActions?.length || detailRows.length)
  const toneCls =
    tone === 'error'
      ? 'border-l-danger text-danger'
      : tone === 'warn'
        ? 'border-l-warn text-warn'
        : 'border-l-rule text-ink-faint'
  return (
    <article
      data-message-role="system-notice"
      data-message-tone={tone}
      className={cn(
        'border-l-2 pl-3 py-1 font-mono text-[12px]',
        structured ? 'tracking-normal' : 'uppercase tracking-[0.04em]',
        toneCls,
      )}
    >
      <div data-message-summary>{message.content}</div>
      {message.nextActions?.length ? (
        <div
          data-message-next-actions
          className="mt-2 text-ink-faint normal-case"
        >
          <div className="text-[10px] uppercase tracking-[0.08em]">
            What you can do
          </div>
          <ul className="mt-1 list-disc space-y-1 pl-4">
            {message.nextActions.map((action) => (
              <li key={action}>{action}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {detailRows.length > 0 ? (
        <details
          data-message-technical-details
          className="mt-2 text-ink-faint normal-case"
        >
          <summary className="cursor-pointer select-none text-[10px] uppercase tracking-[0.08em] hover:text-ink">
            Technical details
          </summary>
          <dl className="mt-2 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1 border-l border-rule-2 pl-3 text-[11px]">
            {detailRows.map(([key, value]) => (
              <div key={key} className="contents">
                <dt className="uppercase text-ink-ghost">{key}</dt>
                <dd
                  data-technical-detail={key}
                  className="break-words text-ink-faint"
                >
                  {value}
                </dd>
              </div>
            ))}
          </dl>
        </details>
      ) : null}
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

        <div className="h-px bg-edge/40" style={{ width: 'calc(100% + 24px)', marginLeft: '-12px' }}></div>

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

/**
 * The `/command` token inside the user bubble: the typed command fused with
 * its expansion metadata (body size), so the slash chip never repeats the
 * same name one row below the bubble. Same anatomy as FunctionMentionPill —
 * accent glyph, ink text, one alpha-surface step above the bubble.
 */
function SlashCommandToken({ chip }: { chip: Attachment }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 px-1.5 h-[22px] rounded-xs bg-surface font-mono text-[13px] text-ink select-none"
      title={`skill body attached · ${formatSize(chip.size)}`}
    >
      <Blocks size={16} aria-hidden className="text-accent shrink-0" />
      <span className="leading-none truncate">{chip.name}</span>
      <span className="leading-none text-[11px] text-ink-ghost tabular-nums shrink-0">
        {formatSize(chip.size)}
      </span>
    </span>
  )
}

function UserMessage({ message }: { message: UserMessageType }) {
  const attachments = message.attachments ?? []
  /* A slash expansion's chip duplicates the command that already leads the
     typed text — fuse it into the bubble as one token instead of orphaning
     it in the strip below. A chip that doesn't match the leading token
     (shouldn't happen) keeps the strip as a fallback. */
  const slashChip = attachments.find(
    (a) =>
      a.type === 'text/x-skill' &&
      message.content.startsWith(a.name) &&
      (message.content.length === a.name.length ||
        message.content.charAt(a.name.length) === ' '),
  )
  const chips = slashChip
    ? attachments.filter((a) => a.id !== slashChip.id)
    : attachments
  const args = slashChip
    ? message.content.slice(slashChip.name.length).trim()
    : ''
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
      <div
        className={cn(
          'max-w-[92%] rounded-sm bg-surface px-3.5 py-2.5 sm:max-w-[80%]',
          'break-words',
          slashChip && 'flex flex-col items-start gap-1.5',
        )}
      >
        {slashChip ? (
          <>
            <SlashCommandToken chip={slashChip} />
            {args ? (
              <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
                {args}
              </Markdown>
            ) : null}
          </>
        ) : (
          <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
            {message.content}
          </Markdown>
        )}
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
      <header className="flex flex-wrap items-center gap-2 font-sans text-base text-ink-ghost sm:text-sm">
        <span className="font-medium text-ink-faint">Agent</span>
        {copySource !== undefined && !message.streaming ? (
          <CopyMessageButton
            text={copySource}
            className="opacity-100 sm:order-last sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
          />
        ) : null}
        {message.model ? (
          <span className="text-ink-ghost">· {message.model}</span>
        ) : null}
        {message.mode ? (
          <span className="text-ink-ghost">· {message.mode}</span>
        ) : null}
        {message.memory ? <MemoryChip memory={message.memory} /> : null}
      </header>
      <div className="pr-1">
        {message.content ? (
          <Markdown className="max-sm:[&_ol]:text-base max-sm:[&_p]:text-base max-sm:[&_ul]:text-base">
            {message.content}
          </Markdown>
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
