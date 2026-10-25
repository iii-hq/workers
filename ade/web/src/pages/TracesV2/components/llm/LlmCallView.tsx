/**
 * The span panel's model-call view: what was asked of the model and what
 * came back, laid out to be read rather than parsed.
 *
 *   facts    — model · provider · stop reason · tokens in/out · cache · cost
 *   request  — the system prompt (collapsed), each message as a role-tagged
 *              block (text as text, tool calls and results as highlighted
 *              JSON), the tool list, the remaining request parameters
 *   response — the returned content, the bookkeeping fields, the error
 *   raw      — both payloads as highlighted JSON with copy buttons
 *
 * Data comes from `lib/llmCall.ts`, which merges the span's `gen_ai.*`
 * attributes with the captured request/response and repairs the engine's
 * 64 KB truncation so a long chat turn still reads as structure.
 */

import {
  AlertCircle,
  Bot,
  ChevronRight,
  Cpu,
  FileText,
  MessageSquare,
  Terminal,
  User,
  Wrench,
} from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@/components/shadcn/tabs'
import { cn } from '@/lib/utils'
import {
  formatCost,
  formatTokenCount,
  type LlmCall,
  type LlmContentBlock,
  type LlmMessage,
} from '../../lib/llmCall'
import { formatDuration } from '../../lib/traceUtils'
import { PayloadPane } from './PayloadPane'

export interface LlmCallViewProps {
  call: LlmCall
  functionId: string | null
  durationMs: number
  status: 'ok' | 'error' | 'pending' | 'unset' | string
  /** The error output (`{ error }`) when the call failed. */
  error?: unknown
}

/** Characters of a text block shown before it folds behind "Show more". */
const TEXT_FOLD_CHARS = 900

function Fact({
  label,
  value,
  mono = true,
  tone,
  title,
}: {
  label: string
  value: ReactNode
  mono?: boolean
  tone?: 'alert' | 'ok'
  title?: string
}) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5" title={title}>
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <span
        className={cn(
          'truncate text-[13px] text-foreground tabular-nums',
          mono && 'font-mono text-[13px]',
          tone === 'alert' && 'text-alert-strong',
          tone === 'ok' && 'text-ok',
        )}
      >
        {value}
      </span>
    </div>
  )
}

function Section({
  title,
  count,
  children,
  defaultOpen = true,
  action,
}: {
  title: string
  count?: number | string
  children: ReactNode
  defaultOpen?: boolean
  action?: ReactNode
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <section className="flex flex-col gap-2">
      <div className="flex h-6 items-center gap-1.5">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          className="-ml-1 flex min-w-0 items-center gap-1 rounded-md px-1 text-left text-xs font-semibold text-foreground transition-colors hover:bg-muted"
        >
          <ChevronRight
            className={cn(
              'size-4 text-muted-foreground transition-transform duration-[var(--motion-duration-control)]',
              open && 'rotate-90',
            )}
            aria-hidden
          />
          <span>{title}</span>
          {count !== undefined ? (
            <span className="font-mono text-[11px] font-normal text-muted-foreground tabular-nums">
              {count}
            </span>
          ) : null}
        </button>
        <span className="ml-auto flex items-center gap-1">{action}</span>
      </div>
      {open ? children : null}
    </section>
  )
}

const ROLE_STYLE: Record<string, { icon: typeof User; tone: string }> = {
  user: { icon: User, tone: 'text-sky-600 dark:text-sky-400' },
  assistant: { icon: Bot, tone: 'text-foreground' },
  system: { icon: FileText, tone: 'text-muted-foreground' },
  tool: { icon: Wrench, tone: 'text-warn-strong' },
  function: { icon: Wrench, tone: 'text-warn-strong' },
}

function FoldedText({ text, className }: { text: string; className?: string }) {
  const [open, setOpen] = useState(false)
  const long = text.length > TEXT_FOLD_CHARS
  const shown =
    long && !open ? `${text.slice(0, TEXT_FOLD_CHARS).trimEnd()}…` : text
  return (
    <div className={cn('min-w-0', className)}>
      <p className="font-sans text-[13px] leading-[1.55] break-words whitespace-pre-wrap text-foreground">
        {shown}
      </p>
      {long ? (
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          className="mt-1 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground"
        >
          {open
            ? 'Show less'
            : `Show more · ${text.length.toLocaleString('en-US')} characters`}
        </button>
      ) : null}
    </div>
  )
}

function Block({ block }: { block: LlmContentBlock }) {
  switch (block.type) {
    case 'text':
      return <FoldedText text={block.text ?? ''} />
    case 'thinking':
    case 'reasoning':
      return (
        <div className="rounded-md border border-dashed border-border px-2.5 py-1.5">
          <span className="text-[11px] font-medium text-muted-foreground">
            thinking
          </span>
          <FoldedText
            text={block.text ?? ''}
            className="mt-0.5 text-muted-foreground"
          />
        </div>
      )
    case 'tool_use':
    case 'function_call':
    case 'tool_call':
      return (
        <PayloadPane
          label={`call ${block.name ?? 'tool'}`}
          hint={block.id}
          value={block.input ?? {}}
        />
      )
    case 'tool_result':
    case 'function_result': {
      const content = (block.raw as { content?: unknown })?.content
      const text =
        typeof content === 'string'
          ? content
          : Array.isArray(content)
            ? content
                .map((c) =>
                  typeof c === 'string'
                    ? c
                    : ((c as { text?: string })?.text ?? ''),
                )
                .filter(Boolean)
                .join('\n')
            : undefined
      return (
        <PayloadPane
          label={block.isError ? 'tool result · error' : 'tool result'}
          hint={block.toolUseId}
          value={text ?? content ?? block.raw}
          language="text"
        />
      )
    }
    case 'image':
      return (
        <span className="text-[11px] text-muted-foreground">[image block]</span>
      )
    default:
      return <PayloadPane label={block.type} value={block.raw} />
  }
}

function Message({ message, index }: { message: LlmMessage; index: number }) {
  const style = ROLE_STYLE[message.role] ?? {
    icon: MessageSquare,
    tone: 'text-foreground',
  }
  const Icon = style.icon
  const summary = [
    message.model,
    message.stopReason ? `stop: ${message.stopReason}` : null,
    message.usage?.input !== undefined
      ? `${formatTokenCount(message.usage.input)} in`
      : null,
    message.usage?.output !== undefined
      ? `${formatTokenCount(message.usage.output)} out`
      : null,
  ].filter(Boolean)
  return (
    <div
      className="flex min-w-0 gap-2.5 py-2"
      data-llm-message-role={message.role}
      data-llm-message-index={index}
    >
      <span
        className={cn(
          'mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-md bg-muted',
          style.tone,
        )}
        aria-hidden
      >
        <Icon className="size-4" />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className={cn('text-xs font-semibold', style.tone)}>
            {message.role}
          </span>
          {summary.length > 0 ? (
            <span className="truncate font-mono text-[11px] text-muted-foreground">
              {summary.join(' · ')}
            </span>
          ) : null}
        </div>
        {message.blocks.length === 0 ? (
          <span className="text-[11px] text-muted-foreground">empty</span>
        ) : (
          message.blocks.map((block, i) => (
            <Block key={`${block.type}:${block.id ?? i}`} block={block} />
          ))
        )}
      </div>
    </div>
  )
}

export function LlmCallView({
  call,
  functionId,
  durationMs,
  status,
  error,
}: LlmCallViewProps) {
  const [tab, setTab] = useState<'conversation' | 'raw'>('conversation')
  const usage = call.usage
  const failed = status === 'error' || error !== undefined
  const errorText = useMemo(() => {
    if (error === undefined) return null
    if (typeof error === 'string') return error
    const e = (error as { error?: unknown })?.error
    if (typeof e === 'string') return e
    if (e && typeof e === 'object') {
      const m = (e as { message?: unknown; kind?: unknown }).message
      const k = (e as { kind?: unknown }).kind
      return [k, m].filter((x) => typeof x === 'string').join(': ') || null
    }
    return null
  }, [error])
  // Messages carry no id: key on role + timestamp + a content digest, with a
  // per-duplicate counter so two identical turns still get distinct keys.
  const messageKeys = useMemo(() => {
    const seen = new Map<string, number>()
    return call.messages.map((m) => {
      const digest = m.blocks
        .map((b) => `${b.type}:${(b.text ?? b.name ?? '').slice(0, 24)}`)
        .join('|')
      const base = `${m.role}:${m.timestamp ?? ''}:${digest}`
      const n = (seen.get(base) ?? 0) + 1
      seen.set(base, n)
      return `${base}#${n}`
    })
  }, [call.messages])
  const hasParams = Object.keys(call.params).length > 0
  const hasResponseFields = Object.keys(call.responseFields).length > 0
  const requestEmpty =
    call.messages.length === 0 && !call.systemPrompt && call.tools.length === 0

  return (
    <div className="flex flex-col gap-4" data-llm-call-view>
      {/* facts */}
      <div className="grid grid-cols-2 gap-x-4 gap-y-3 rounded-lg border border-border bg-muted/30 px-3 py-2.5 @md:grid-cols-3 @xl:grid-cols-4">
        <div className="col-span-2 flex min-w-0 flex-col gap-0.5 @md:col-span-3 @xl:col-span-4">
          <span className="text-[11px] text-muted-foreground">Model</span>
          <span className="flex min-w-0 items-center gap-2">
            <Cpu
              className="size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <span className="truncate font-mono text-[13px] font-medium text-foreground">
              {call.model ?? '—'}
            </span>
            {call.provider ? (
              <span className="shrink-0 rounded-md bg-muted px-1.5 py-px text-[11px] text-muted-foreground">
                {call.provider}
              </span>
            ) : null}
            {functionId ? (
              <span className="hidden min-w-0 truncate font-mono text-[11px] text-muted-foreground @lg:inline">
                {functionId}
              </span>
            ) : null}
          </span>
        </div>
        <Fact
          label="Status"
          value={failed ? 'error' : status === 'pending' ? 'running' : status}
          tone={failed ? 'alert' : status === 'ok' ? 'ok' : undefined}
        />
        <Fact label="Duration" value={formatDuration(durationMs)} />
        <Fact label="Stop reason" value={call.stopReason ?? '—'} />
        <Fact
          label="Tokens in / out"
          value={
            usage?.input !== undefined || usage?.output !== undefined
              ? `${usage?.input !== undefined ? formatTokenCount(usage.input) : '—'} / ${usage?.output !== undefined ? formatTokenCount(usage.output) : '—'}`
              : '—'
          }
          title={
            usage
              ? `${usage.input?.toLocaleString('en-US') ?? '—'} input · ${usage.output?.toLocaleString('en-US') ?? '—'} output tokens`
              : undefined
          }
        />
        {usage?.cacheRead !== undefined || usage?.cacheWrite !== undefined ? (
          <Fact
            label="Cache read / write"
            value={`${usage.cacheRead !== undefined ? formatTokenCount(usage.cacheRead) : '—'} / ${usage.cacheWrite !== undefined ? formatTokenCount(usage.cacheWrite) : '—'}`}
          />
        ) : null}
        {usage?.costUsd !== undefined ? (
          <Fact label="Cost" value={formatCost(usage.costUsd)} />
        ) : null}
      </div>

      {errorText ? (
        <div
          role="alert"
          className="flex items-start gap-2 rounded-lg border border-alert/30 bg-alert-muted px-3 py-2 text-[13px] text-foreground"
        >
          <AlertCircle
            className="mt-0.5 size-4 shrink-0 text-alert"
            aria-hidden
          />
          <span className="min-w-0 break-words whitespace-pre-wrap">
            {errorText}
          </span>
        </div>
      ) : null}

      <Tabs
        value={tab}
        onValueChange={(v) => setTab(v as 'conversation' | 'raw')}
        className="gap-3"
      >
        <TabsList
          variant="line"
          className="h-7 w-full justify-start border-b border-border"
        >
          <TabsTrigger value="conversation" className="flex-none px-1 text-xs">
            <MessageSquare data-icon="inline-start" className="size-4" />
            Conversation
          </TabsTrigger>
          <TabsTrigger value="raw" className="flex-none px-1 text-xs">
            <Terminal data-icon="inline-start" className="size-4" />
            Raw JSON
          </TabsTrigger>
        </TabsList>

        <TabsContent value="conversation" className="flex flex-col gap-4">
          {requestEmpty ? (
            <p className="text-[13px] text-muted-foreground">
              No request was captured for this span.
            </p>
          ) : null}

          {call.systemPrompt ? (
            <Section title="System prompt" defaultOpen={false}>
              <PayloadPane
                label="system"
                value={call.systemPrompt}
                language="markdown"
                hint={`${call.systemPrompt.length.toLocaleString('en-US')} characters`}
              />
            </Section>
          ) : null}

          {call.messages.length > 0 ? (
            <Section title="Messages" count={call.messages.length}>
              <div className="divide-y divide-border rounded-lg border border-border px-3">
                {call.messages.map((message, i) => (
                  <Message key={messageKeys[i]} message={message} index={i} />
                ))}
              </div>
            </Section>
          ) : null}

          <Section
            title="Response"
            action={
              call.responseTruncatedBytes ? (
                <span className="text-[11px] text-warn-strong">truncated</span>
              ) : null
            }
          >
            {call.responseBlocks.length > 0 ? (
              <div className="rounded-lg border border-border px-3">
                <Message
                  message={{ role: 'assistant', blocks: call.responseBlocks }}
                  index={-1}
                />
              </div>
            ) : null}
            {hasResponseFields ? (
              <PayloadPane label="response" value={call.responseFields} />
            ) : null}
            {call.responseBlocks.length === 0 && !hasResponseFields ? (
              <p className="text-[13px] text-muted-foreground">
                {status === 'pending'
                  ? 'Still streaming — the response lands when the call completes.'
                  : call.response === undefined
                    ? 'No response was captured for this span.'
                    : 'The response carried only the facts above.'}
              </p>
            ) : null}
          </Section>

          {call.tools.length > 0 ? (
            <Section
              title="Tools"
              count={call.tools.length}
              defaultOpen={false}
            >
              <div className="divide-y divide-border rounded-lg border border-border">
                {call.tools.map((tool) => (
                  <ToolRow key={tool.name} tool={tool} />
                ))}
              </div>
            </Section>
          ) : null}

          {hasParams ? (
            <Section
              title="Parameters"
              count={Object.keys(call.params).length}
              defaultOpen={false}
            >
              <PayloadPane label="parameters" value={call.params} />
            </Section>
          ) : null}
        </TabsContent>

        <TabsContent value="raw" className="flex flex-col gap-3">
          <PayloadPane
            label="request"
            value={call.request ?? {}}
            truncatedBytes={call.requestTruncatedBytes}
            hint={
              call.requestTruncatedBytes ? 'repaired for display' : undefined
            }
          />
          <PayloadPane
            label="response"
            value={call.response ?? error ?? {}}
            truncatedBytes={call.responseTruncatedBytes}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function ToolRow({
  tool,
}: {
  tool: { name: string; description?: string; parameters?: unknown }
}) {
  const [open, setOpen] = useState(false)
  return (
    <div className="flex flex-col">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex min-w-0 items-start gap-2 px-3 py-2 text-left transition-colors hover:bg-muted/60"
      >
        <ChevronRight
          className={cn(
            'mt-0.5 size-4 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-duration-control)]',
            open && 'rotate-90',
          )}
          aria-hidden
        />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="font-mono text-xs font-medium text-foreground">
            {tool.name}
          </span>
          {tool.description ? (
            <span
              className={cn(
                'text-[12px] leading-[1.5] text-muted-foreground',
                !open && 'line-clamp-2',
              )}
            >
              {tool.description}
            </span>
          ) : null}
        </span>
      </button>
      {open && tool.parameters !== undefined ? (
        <div className="px-3 pb-3">
          <PayloadPane label="schema" value={tool.parameters} />
        </div>
      ) : null}
    </div>
  )
}
